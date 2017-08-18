//! Command and status busses shared between components and threads

use multiqueue;
use std::time::Instant;


#[derive(Clone)]
pub struct Bus {
    pub sender: multiqueue::BroadcastSender<TimestampedMessage>,
    pub receiver: multiqueue::BroadcastReceiver<TimestampedMessage>,
}

impl Bus {
    pub fn new() -> Bus {
        let (sender, receiver) = multiqueue::broadcast_queue(512);
        Bus { sender, receiver }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Command {
    SetMode(ControllerMode),
    ManualControlReset,
    ManualControlValue(ManualControlAxis, f32)
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimestampedMessage {
    pub timestamp: Instant,
    pub message: Message,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Message {
    Command(Command),
    FlyerSensors(FlyerSensors),
    WinchStatus(usize, WinchStatus),
}

impl Message {
    pub fn timestamp(self) -> TimestampedMessage {
        TimestampedMessage {
            timestamp: Instant::now(),
            message: self
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ControllerMode {
    Halted,
    Manual,
    Normal,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ManualControlAxis {
    CameraYaw,
    CameraPitch,
    RelativeX,
    RelativeY,
    RelativeZ,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct XBandTelemetry {
    pub edge_count: u32,
    pub speed_measure: u32,
    pub measure_count: u32,
}

const NUM_LIDAR_SENSORS : usize = 4;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LIDARTelemetry {
    pub ranges: [u32; NUM_LIDAR_SENSORS],
    pub counters: [u32; NUM_LIDAR_SENSORS],
}

const NUM_ANALOG_SENSORS : usize = 8;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AnalogTelemetry {
    pub values: [u32; NUM_ANALOG_SENSORS],
    pub counter: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Vec16 {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Quat16 {
    pub w: i16,
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IMUTelemetry {
    pub accelerometer: Vec16,
    pub magnetometer: Vec16,
    pub gyroscope: Vec16,
    pub euler_angles: Vec16,
    pub quaternion: Quat16,
    pub linear_accel: Vec16,
    pub gravity: Vec16,
    pub temperature: i8,
    pub calib_stat: i8,
    pub counter: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FlyerSensors {
    pub xband: XBandTelemetry,
    pub lidar: LIDARTelemetry,
    pub analog: AnalogTelemetry,
    pub imu: IMUTelemetry,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ForceTelemetry {
    pub measure: i32,           // Uncalibrated, (+) = increasing tension
    pub filtered: f32,          // Same units, just low-pass filtered prior to limit testing
    pub counter: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WinchCommand {
    pub velocity_target: f32,   // Encoder position units per second
    pub accel_max: f32,         // Encoder units per second per second, peak
    pub force_min: f32,         // Uncalibrated load cell units, no negative motion below
    pub force_max: f32,         // Uncalibrated load cell unitsNo positive motion above this filtered force value
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WinchSensors {
    pub force: ForceTelemetry,
    pub position: i32,          // Integrated position in encoder units, from hardware
    pub velocity: i32,          // Instantaneous velocity in encoder units per tick, from hardware
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WinchMotorControl {
    pub pwm: i32,
    pub ramp_velocity: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WinchStatus {
    pub command_counter: u32,
    pub tick_counter: u32,
    pub command: WinchCommand,
    pub sensors: WinchSensors,
    pub motor: WinchMotorControl
}
