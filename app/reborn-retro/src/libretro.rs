//! The parts of the libretro API this frontend uses, declared after
//! libretro.h, and a core opened with dlopen. libretro.h's licence covers
//! these declarations:
//!
//! Copyright (C) 2010-2020 The RetroArch team
//!
//! Permission is hereby granted, free of charge, to any person obtaining a
//! copy of this software and associated documentation files (the
//! "Software"), to deal in the Software without restriction, including
//! without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to permit
//! persons to whom the Software is furnished to do so, subject to the
//! following conditions:
//!
//! The above copyright notice and this permission notice shall be included
//! in all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
//! THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
//! FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
//! DEALINGS IN THE SOFTWARE.
use std::{
    ffi::{c_char, c_uint, c_void, CStr, CString},
    path::Path,
};

pub const API_VERSION: c_uint = 1;
pub const DEVICE_JOYPAD: c_uint = 1;
pub const DEVICE_ID_JOYPAD_MASK: c_uint = 256;
pub const MEMORY_SAVE_RAM: c_uint = 0;

const EXPERIMENTAL: c_uint = 0x10000;
pub const ENV_SET_ROTATION: c_uint = 1;
pub const ENV_GET_OVERSCAN: c_uint = 2;
pub const ENV_GET_CAN_DUPE: c_uint = 3;
pub const ENV_SET_MESSAGE: c_uint = 6;
pub const ENV_SHUTDOWN: c_uint = 7;
pub const ENV_SET_PERFORMANCE_LEVEL: c_uint = 8;
pub const ENV_GET_SYSTEM_DIRECTORY: c_uint = 9;
pub const ENV_SET_PIXEL_FORMAT: c_uint = 10;
pub const ENV_SET_INPUT_DESCRIPTORS: c_uint = 11;
pub const ENV_GET_VARIABLE: c_uint = 15;
pub const ENV_SET_VARIABLES: c_uint = 16;
pub const ENV_GET_VARIABLE_UPDATE: c_uint = 17;
pub const ENV_SET_SUPPORT_NO_GAME: c_uint = 18;
pub const ENV_GET_LIBRETRO_PATH: c_uint = 19;
pub const ENV_GET_CORE_ASSETS_DIRECTORY: c_uint = 30;
pub const ENV_GET_SAVE_DIRECTORY: c_uint = 31;
pub const ENV_SET_SYSTEM_AV_INFO: c_uint = 32;
pub const ENV_SET_SUBSYSTEM_INFO: c_uint = 34;
pub const ENV_SET_CONTROLLER_INFO: c_uint = 35;
pub const ENV_SET_MEMORY_MAPS: c_uint = 36 | EXPERIMENTAL;
pub const ENV_SET_GEOMETRY: c_uint = 37;
pub const ENV_GET_LANGUAGE: c_uint = 39;
pub const ENV_SET_SUPPORT_ACHIEVEMENTS: c_uint = 42 | EXPERIMENTAL;
pub const ENV_SET_SERIALIZATION_QUIRKS: c_uint = 44;
pub const ENV_GET_AUDIO_VIDEO_ENABLE: c_uint = 47 | EXPERIMENTAL;
pub const ENV_GET_FASTFORWARDING: c_uint = 49 | EXPERIMENTAL;
pub const ENV_GET_INPUT_BITMASKS: c_uint = 51 | EXPERIMENTAL;
pub const ENV_GET_CORE_OPTIONS_VERSION: c_uint = 52;
pub const ENV_GET_MESSAGE_INTERFACE_VERSION: c_uint = 59;
pub const ENV_GET_INPUT_MAX_USERS: c_uint = 61;
pub const ENV_SET_MINIMUM_AUDIO_LATENCY: c_uint = 63;
pub const ENV_SET_CONTENT_INFO_OVERRIDE: c_uint = 65;

#[repr(C)]
pub struct SystemInfo {
    pub library_name: *const c_char,
    pub library_version: *const c_char,
    pub valid_extensions: *const c_char,
    pub need_fullpath: bool,
    pub block_extract: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GameGeometry {
    pub base_width: c_uint,
    pub base_height: c_uint,
    pub max_width: c_uint,
    pub max_height: c_uint,
    pub aspect_ratio: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SystemTiming {
    pub fps: f64,
    pub sample_rate: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SystemAvInfo {
    pub geometry: GameGeometry,
    pub timing: SystemTiming,
}

#[repr(C)]
pub struct GameInfo {
    pub path: *const c_char,
    pub data: *const c_void,
    pub size: usize,
    pub meta: *const c_char,
}

#[repr(C)]
pub struct Variable {
    pub key: *const c_char,
    pub value: *const c_char,
}

#[repr(C)]
pub struct Message {
    pub msg: *const c_char,
    pub frames: c_uint,
}

pub type EnvironmentFn = unsafe extern "C" fn(cmd: c_uint, data: *mut c_void) -> bool;
pub type VideoRefreshFn =
    unsafe extern "C" fn(data: *const c_void, width: c_uint, height: c_uint, pitch: usize);
pub type AudioSampleFn = unsafe extern "C" fn(left: i16, right: i16);
pub type AudioSampleBatchFn = unsafe extern "C" fn(data: *const i16, frames: usize) -> usize;
pub type InputPollFn = unsafe extern "C" fn();
pub type InputStateFn =
    unsafe extern "C" fn(port: c_uint, device: c_uint, index: c_uint, id: c_uint) -> i16;

/// A loaded core: its entry points. The library stays loaded for the life
/// of the process (cores are not unloaded; the process exits instead).
pub struct Core {
    pub set_environment: unsafe extern "C" fn(EnvironmentFn),
    pub set_video_refresh: unsafe extern "C" fn(VideoRefreshFn),
    pub set_audio_sample: unsafe extern "C" fn(AudioSampleFn),
    pub set_audio_sample_batch: unsafe extern "C" fn(AudioSampleBatchFn),
    pub set_input_poll: unsafe extern "C" fn(InputPollFn),
    pub set_input_state: unsafe extern "C" fn(InputStateFn),
    pub init: unsafe extern "C" fn(),
    pub deinit: unsafe extern "C" fn(),
    pub api_version: unsafe extern "C" fn() -> c_uint,
    pub get_system_info: unsafe extern "C" fn(*mut SystemInfo),
    pub get_system_av_info: unsafe extern "C" fn(*mut SystemAvInfo),
    pub set_controller_port_device: unsafe extern "C" fn(c_uint, c_uint),
    pub run: unsafe extern "C" fn(),
    pub serialize_size: unsafe extern "C" fn() -> usize,
    pub serialize: unsafe extern "C" fn(*mut c_void, usize) -> bool,
    pub unserialize: unsafe extern "C" fn(*const c_void, usize) -> bool,
    pub load_game: unsafe extern "C" fn(*const GameInfo) -> bool,
    pub unload_game: unsafe extern "C" fn(),
    pub get_memory_data: unsafe extern "C" fn(c_uint) -> *mut c_void,
    pub get_memory_size: unsafe extern "C" fn(c_uint) -> usize,
}

fn dl_error() -> String {
    // SAFETY: dlerror returns NULL or a C string valid until the next dl
    // call on this thread.
    unsafe {
        let e = libc::dlerror();
        if e.is_null() {
            "unknown error".into()
        } else {
            CStr::from_ptr(e).to_string_lossy().into_owned()
        }
    }
}

impl Core {
    pub fn open(path: &Path) -> Result<Self, String> {
        let name = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_| "core path")?;
        // SAFETY: a valid C string; the handle is kept for the process's
        // life.
        let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err(format!("the core could not be opened ({})", dl_error()));
        }
        let sym = |n: &str| -> Result<*mut c_void, String> {
            let c = CString::new(n).unwrap_or_default();
            // SAFETY: a live handle and a valid symbol name.
            let p = unsafe { libc::dlsym(handle, c.as_ptr()) };
            if p.is_null() {
                Err(format!("the core has no {n}"))
            } else {
                Ok(p)
            }
        };
        macro_rules! f {
            ($n:literal) => {
                // SAFETY: the symbol is the libretro entry point of this name,
                // whose C signature the field's type spells out.
                unsafe { std::mem::transmute_copy::<*mut c_void, _>(&sym($n)?) }
            };
        }
        Ok(Self {
            set_environment: f!("retro_set_environment"),
            set_video_refresh: f!("retro_set_video_refresh"),
            set_audio_sample: f!("retro_set_audio_sample"),
            set_audio_sample_batch: f!("retro_set_audio_sample_batch"),
            set_input_poll: f!("retro_set_input_poll"),
            set_input_state: f!("retro_set_input_state"),
            init: f!("retro_init"),
            deinit: f!("retro_deinit"),
            api_version: f!("retro_api_version"),
            get_system_info: f!("retro_get_system_info"),
            get_system_av_info: f!("retro_get_system_av_info"),
            set_controller_port_device: f!("retro_set_controller_port_device"),
            run: f!("retro_run"),
            serialize_size: f!("retro_serialize_size"),
            serialize: f!("retro_serialize"),
            unserialize: f!("retro_unserialize"),
            load_game: f!("retro_load_game"),
            unload_game: f!("retro_unload_game"),
            get_memory_data: f!("retro_get_memory_data"),
            get_memory_size: f!("retro_get_memory_size"),
        })
    }
}
