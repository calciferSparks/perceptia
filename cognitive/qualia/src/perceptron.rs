// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of
// the MPL was not distributed with this file, You can obtain one at http://mozilla.org/MPL/2.0/

//! Definitions of events used in whole application.
//!
//! TODO: Perceptron is `perceptia`-specific and should be moved out of `cognitive`.

// -------------------------------------------------------------------------------------------------

use std;
use std::os::unix::io::RawFd;

use dharma::SignalId;

use timing::Milliseconds;
use enums::{ClientChange, InteractionMode};
use defs::{Command, SurfaceId, Position, OptionalPosition, Vector, Size};
use input::{Axis, Button, Key};
use output::{OutputInfo, OutputType};

// -------------------------------------------------------------------------------------------------

pub const NOTIFY: SignalId = 0;
pub const SUSPEND: SignalId = 1;
pub const WAKEUP: SignalId = 2;
pub const VERTICAL_BLANK: SignalId = 3;
pub const PAGE_FLIP: SignalId = 4;
pub const INPUTS_CHANGED: SignalId = 5;
pub const OUTPUTS_CHANGED: SignalId = 6;
pub const OUTPUT_FOUND: SignalId = 7;
pub const DISPLAY_CREATED: SignalId = 9;
pub const INPUT_POINTER_MOTION: SignalId = 10;
pub const INPUT_POINTER_POSITION: SignalId = 11;
pub const INPUT_POINTER_BUTTON: SignalId = 12;
pub const INPUT_POINTER_AXIS: SignalId = 13;
pub const INPUT_POINTER_POSITION_RESET: SignalId = 14;
pub const INPUT_KEYBOARD: SignalId = 15;
pub const SURFACE_READY: SignalId = 20;
pub const SURFACE_DESTROYED: SignalId = 21;
pub const SURFACE_RECONFIGURED: SignalId = 22;
pub const DOCK_SURFACE: SignalId = 23;
pub const CURSOR_SURFACE_CHANGE: SignalId = 25;
pub const BACKGROUND_SURFACE_CHANGE: SignalId = 26;
pub const SURFACE_FRAME: SignalId = 30;
pub const POINTER_FOCUS_CHANGED: SignalId = 31;
pub const POINTER_RELATIVE_MOTION: SignalId = 32;
pub const KEYBOARD_FOCUS_CHANGED: SignalId = 33;
pub const TRANSFER_OFFERED: SignalId = 41;
pub const TRANSFER_REQUESTED: SignalId = 42;
pub const COMMAND: SignalId = 51;
pub const MODE: SignalId = 52;
pub const TAKE_SCREENSHOT: SignalId = 101;
pub const SCREENSHOT_DONE: SignalId = 102;
pub const WORKSPACE_STATE_CHANGED: SignalId = 111;
pub const TIMER_500: SignalId = 121;
pub const REMOTE_CLIENT_CHANGE: SignalId = 131;

// -------------------------------------------------------------------------------------------------

/// Data passed along with signals. Convention it to use enum values only with corresponding signal
/// identifies.
///
/// TODO: Describe all `perceptrons`.
#[repr(C)]
#[derive(Clone)]
pub enum Perceptron {
    Notify,
    Suspend,
    WakeUp,
    InputsChanged,
    OutputsChanged,
    CustomEmpty,
    CustomId(u64),
    VerticalBlank(i32),
    PageFlip(i32),
    OutputFound(OutputType),
    DisplayCreated(OutputInfo),
    InputPointerMotion(Vector),
    InputPointerPosition(OptionalPosition),
    InputPointerButton(Button),
    InputPointerAxis(Axis),
    InputPointerPositionReset(Option<Position>),
    InputKeyboard(Key),
    SurfaceReady(SurfaceId),
    SurfaceDestroyed(SurfaceId),
    SurfaceReconfigured(SurfaceId),
    DockSurface(SurfaceId, Size, i32),
    CursorSurfaceChange(SurfaceId),
    BackgroundSurfaceChange(SurfaceId),
    SurfaceFrame(SurfaceId, Milliseconds),
    PointerFocusChanged(SurfaceId, SurfaceId, Position),
    PointerRelativeMotion(SurfaceId, Position, Milliseconds),
    KeyboardFocusChanged(SurfaceId, SurfaceId),
    TransferOffered,
    TransferRequested(String, RawFd),
    Command(Command),
    Mode { active: bool, mode: InteractionMode },
    TakeScreenshot(i32),
    ScreenshotDone,
    WorkspaceStateChanged,
    Timer500,
    RemoteClientChange(ClientChange),
}

// -------------------------------------------------------------------------------------------------

impl std::fmt::Debug for Perceptron {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            Perceptron::Notify => write!(f, "Notify"),
            Perceptron::Suspend => write!(f, "Suspend"),
            Perceptron::WakeUp => write!(f, "WakeUp"),
            Perceptron::InputsChanged => write!(f, "InputsChanged"),
            Perceptron::OutputsChanged => write!(f, "OutputsChanged"),
            Perceptron::CustomEmpty => write!(f, "CustomEmpty"),
            Perceptron::CustomId(ref id) => write!(f, "CustomId({:?})", id),
            Perceptron::VerticalBlank(ref data) => write!(f, "VerticalBlank({:?})", data),
            Perceptron::PageFlip(ref data) => write!(f, "PageFlip({:?})", data),
            Perceptron::OutputFound(ref bundle) => write!(f, "OutputFound({:?})", bundle),
            Perceptron::DisplayCreated(ref info) => write!(f, "DisplayCreated({:?})", info),
            Perceptron::InputPointerMotion(ref vector) => {
                write!(f, "InputPointerMotion({:?})", vector)
            }
            Perceptron::InputPointerPosition(ref pos) => {
                write!(f, "InputPointerPosition({:?})", pos)
            }
            Perceptron::InputPointerButton(ref btn) => write!(f, "InputPointerButton({:?})", btn),
            Perceptron::InputPointerAxis(ref axis) => write!(f, "InputPointerAxis({:?})", axis),
            Perceptron::InputPointerPositionReset(ref position) => {
                write!(f, "InputPointerPositionReset({:?})", position)
            }
            Perceptron::InputKeyboard(ref key) => write!(f, "InputKeyboard({:?})", key),

            Perceptron::SurfaceReady(ref sid) => write!(f, "SurfaceReady({})", sid),
            Perceptron::SurfaceDestroyed(ref sid) => write!(f, "SurfaceDestroyed({})", sid),
            Perceptron::SurfaceReconfigured(ref sid) => write!(f, "SurfaceReconfigured({})", sid),
            Perceptron::DockSurface(ref sid, ref size, display_id) => {
                write!(f, "DockSurface({}, {:?}, {:?})", sid, size, display_id)
            }
            Perceptron::CursorSurfaceChange(ref sid) => write!(f, "CursorSurfaceChange({})", sid),
            Perceptron::BackgroundSurfaceChange(ref sid) => {
                write!(f, "BackgroundSurfaceChange({})", sid)
            }
            Perceptron::SurfaceFrame(sid, time) => {
                write!(f, "SurfaceFrame(sid: {}, milliseconds: {})", sid, time.get_value())
            }
            Perceptron::PointerFocusChanged(ref old_sid, ref new_sid, ref pos) => {
                write!(f, "PointerFocusChanged(old: {:?}, new: {:?}, {:?})", old_sid, new_sid, pos)
            }
            Perceptron::PointerRelativeMotion(ref sid, ref pos, ref time) => {
                write!(f, "PointerRelativeMotion({:?}, {:?}, {:?})", sid, pos, time.get_value())
            }
            Perceptron::KeyboardFocusChanged(ref old_sid, ref new_sid) => {
                write!(f, "KeyboardFocusChanged({:?}, {:?})", old_sid, new_sid)
            }
            Perceptron::TransferOffered => write!(f, "TransferOffered"),
            Perceptron::TransferRequested(ref mime_type, fd) => {
                write!(f, "TransferRequested('{:?}', fd: {:?})", mime_type, fd)
            }
            Perceptron::Command(ref command) => write!(f, "Command({:?})", command),
            Perceptron::Mode { active, mode } => {
                write!(f, "Mode(active: {:?}, mode: {:?})", active, mode)
            }
            Perceptron::TakeScreenshot(ref id) => write!(f, "TakeScreenshot({:?})", id),
            Perceptron::ScreenshotDone => write!(f, "ScreenshotDone"),
            Perceptron::WorkspaceStateChanged => write!(f, "WorkspaceStateChanged"),
            Perceptron::Timer500 => write!(f, "Timer500"),
            Perceptron::RemoteClientChange(change) => write!(f, "RemoteClientChange({:?})", change),
        }
    }
}

// -------------------------------------------------------------------------------------------------
