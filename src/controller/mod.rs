//! Controller thread, responding to commands and status updates, generating motor control packets

mod manual;
mod velocity;
mod winch;
mod state;
mod timer;
mod gimbal;
mod draw;

use message::*;
use vecmath::*;
use std::sync::mpsc::{SyncSender, Receiver, sync_channel};
use bus::{Bus, BusReader};
use config::{SharedConfigFile, Config, ControllerMode};
use botcomm::BotSocket;
use fygimbal::GimbalPort;
use self::state::ControllerState;
use self::timer::{ConfigScheduler, ControllerTimers};
use self::gimbal::GimbalController;
use led::{LightEnvironment, LightAnimator};
use overlay::DrawingContext;

pub struct Controller {
    recv: Receiver<ControllerInput>,
    bus: Bus<TimestampedMessage>,
    port_prototype: ControllerPort,
    socket: BotSocket,
    shared_config: SharedConfigFile,
    local_config: Config,
    state: ControllerState,
    config_scheduler: ConfigScheduler,
    timers: ControllerTimers,
    draw: DrawingContext,
    lights: LightAnimator,
    gimbal_ctrl: GimbalController,
    gimbal_status: Option<GimbalControlStatus>,
}

enum ControllerInput {
    Message(TimestampedMessage),
    ReaderRequest(SyncSender<BusReader<TimestampedMessage>>),
}

#[derive(Clone)]
pub struct ControllerPort {
    sender: SyncSender<ControllerInput>,
}

impl ControllerPort {
    pub fn send(&self, msg: TimestampedMessage) {
        if self.sender.try_send(ControllerInput::Message(msg)).is_err() {
            println!("Controller input queue overflow!");
        }
    }

    pub fn add_rx(&self) -> BusReader<TimestampedMessage> {
        let (result_sender, result_recv) = sync_channel(1);
        drop(self.sender.try_send(ControllerInput::ReaderRequest(result_sender)));
        result_recv.recv().unwrap()
    }
}

impl Controller {
    pub fn new(config: &SharedConfigFile, socket: &BotSocket) -> Controller {

        const DEPTH : usize = 1024;

        let (sender, recv) = sync_channel(DEPTH);
        let bus = Bus::new(DEPTH);
        let port_prototype = ControllerPort { sender };

        let local_config = config.get_latest();
        let lights = LightAnimator::start(&local_config.lighting.animation, &socket);
        let state = ControllerState::new(&local_config);

        Controller {
            lights,
            recv,
            bus,
            port_prototype,
            state,
            local_config,
            socket: socket.try_clone().unwrap(),
            shared_config: config.clone(),
            config_scheduler: ConfigScheduler::new(),
            timers: ControllerTimers::new(),
            draw: DrawingContext::new(),
            gimbal_ctrl: GimbalController::new(),
            gimbal_status: None
        }
    }

    pub fn port(&self) -> ControllerPort {
        self.port_prototype.clone()
    }

    pub fn run(mut self, gimbal_port: GimbalPort) {
        println!("Running.");
        loop {
            self.poll(&gimbal_port);
        }
    }

    fn broadcast(&mut self, ts_msg: TimestampedMessage) {
        if self.bus.try_broadcast(ts_msg).is_err() {
            println!("Controller output bus overflow!");
        }
    }

    fn config_changed(&mut self) {
        self.shared_config.set(self.local_config.clone());
        let msg = Message::ConfigIsCurrent(self.local_config.clone());
        self.broadcast(msg.timestamp());
        self.state.config_changed(&self.local_config);
    }

    fn poll(&mut self, gimbal_port: &GimbalPort) {
        match self.recv.recv().unwrap() {

            ControllerInput::ReaderRequest(result_channel) => {
                // Never blocks, result_channel must already have room
                let rx = self.bus.add_rx();
                drop(result_channel.try_send(rx));
            }

            ControllerInput::Message(ts_msg) => {
                self.broadcast(ts_msg.clone());
                self.handle_message(ts_msg, gimbal_port);
            }
        }

        if self.timers.tick.poll() {
            self.state.every_tick(&self.local_config);
            let light_env = self.light_environment(&self.local_config);
            self.lights.update(light_env);

            let gimbal_status = self.gimbal_ctrl.tick(&self.local_config, gimbal_port, &self.state.tracked);
            let reset_tracking = gimbal_status.current_error_duration > self.local_config.gimbal.error_duration_for_rehome;
            self.gimbal_status = Some(gimbal_status.clone());
            self.broadcast(Message::GimbalControlStatus(gimbal_status).timestamp());

            if let Some(tracking_rect) = self.state.tracking_update(&self.local_config, 1.0 / TICK_HZ as f32, reset_tracking) {
                self.broadcast(Message::CameraInitTrackedRegion(tracking_rect).timestamp());
            }
        }

        if self.timers.video_frame.poll() {
            self.render_overlay();
            let scene = self.draw.scene.drain(..).collect();
            self.broadcast(Message::CameraOverlayScene(scene).timestamp());
        }

        if self.config_scheduler.poll(&mut self.local_config) {
            self.config_changed();
        }
    }

    fn render_overlay(&mut self) {
        let config = &self.local_config;
        self.draw.clear();
        draw::mode_indicator(config, &mut self.draw);
        draw::detected_objects(config, &mut self.draw, &self.state.detected.1);
        draw::tracking_gains(config, &mut self.draw, &self.gimbal_status);
        draw::tracking_rect(config, &mut self.draw, &self.state.tracked, &self.state.manual);
        draw::gimbal_status(config, &mut self.draw, &self.gimbal_status);
        draw::debug_text(config, &mut self.draw, format!("{:?}, {:?}", config.mode, self.gimbal_status));

        match config.mode {
            ControllerMode::Halted => {},
            _ => {
                self.state.tracking_particles.render(config, &mut self.draw);
            }
        }
    }

    fn light_environment(&self, config: &Config) -> LightEnvironment {
        let camera_yaw_angle = if let Some(ref gimbal) = self.gimbal_status {
            gimbal.angles[0] as f32 * TAU / 4096.0
        } else {
            0.0
        };

        let ring_color = if config.mode == ControllerMode::Halted {
            config.lighting.current.flyer_ring_halt_color
        } else if self.state.tracked.age > config.vision.tracking_age_boredom_threshold {
            config.lighting.current.flyer_ring_bored_color
        } else {
            config.lighting.current.flyer_ring_tracking_color
        };

        LightEnvironment {
            config: config.lighting.current.clone(),
            winches: self.state.winch_lighting(config),
            camera_yaw_angle,
            is_recording: self.state.camera_output_is_active(&CameraOutput::LocalRecording),
            is_streaming: self.state.camera_output_is_active(&CameraOutput::LiveStream),
            ring_color,
        }
    }

    fn handle_message(&mut self, ts_msg: TimestampedMessage, gimbal_port: &GimbalPort) {
        match ts_msg.message {

            Message::UpdateConfig(updates) => {
                // Merge a freeform update into the configuration, and save it.
                // Errors here go right to the console, since errors caused by a
                // client should have been detected earlier and sent to that client.
                match self.local_config.merge(updates) {
                    Err(e) => println!("Error in UpdateConfig from message bus: {}", e),
                    Ok(config) => {
                        self.local_config = config;
                        self.config_changed();
                    }
                }
            }

            Message::WinchStatus(id, status) => {
                let command = self.state.winch_control_loop(&self.local_config, id, status);
                if self.state.multi_winch_watchdog_should_halt(&self.local_config) {
                    println!("Halting; lost communication with one or more winches");
                    self.local_config.mode = ControllerMode::Halted;
                    self.config_changed();
                }
                drop(self.socket.winch_command(id, command));
            },

            Message::FlyerSensors(sensors) => {
                self.state.flyer_sensor_update(sensors);
            },

            Message::GimbalValue(val, _) => {
                self.gimbal_ctrl.value_received(&self.local_config, val)
            },

            Message::Command(Command::CameraObjectDetection(obj)) => {
                self.state.camera_object_detection_update(obj);
                if let Some(tracking_rect) = self.state.tracking_update(&self.local_config, 0.0, false) {
                    self.broadcast(Message::CameraInitTrackedRegion(tracking_rect).timestamp());
                }
            },

            Message::Command(Command::CameraRegionTracking(tr)) => {
                self.state.camera_region_tracking_update(tr);
            },

            Message::Command(Command::CameraOutputStatus(outs)) => {
                self.state.camera_output_status_update(outs);
            },

            Message::Command(Command::SetMode(mode)) => {
                // The controller mode is part of the config, so this could be changed via UpdateConfig as well, but this option is strongly typed
                if self.local_config.mode != mode {
                    self.local_config.mode = mode;
                    self.config_changed();
                }
            },

            Message::Command(Command::GimbalMotorEnable(en)) => {
                self.gimbal_ctrl.set_motor_enable(gimbal_port, en);
            },

            Message::Command(Command::GimbalPacket(packet)) => {
                gimbal_port.send_packet(packet);
            },

            Message::Command(Command::GimbalValueWrite(data)) => {
                gimbal_port.write_value(data);
            },

            Message::Command(Command::GimbalValueRequests(reqs)) => {
                gimbal_port.request_values(reqs);
            },

            Message::Command(Command::ManualControlValue(axis, value)) => {
                self.state.manual.control_value(axis, value);
            },

            Message::Command(Command::ManualControlReset) => {
                self.state.manual.control_reset();
            },

            _ => (),
        }
    }
}
