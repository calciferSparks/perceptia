// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of
// the MPL was not distributed with this file, You can obtain one at http://mozilla.org/MPL/2.0/

//! This module contains functionality related to catching key bindings and executing assigned
//! functions.
//!
//! This functionality is inspired by `vim`. As there we have here modes to be able to change
//! applications behavior depending on which modes are on or off.

// -------------------------------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use uinput_sys;

use dharma::Signaler;
use qualia::{Action, Axis, Binding, Button, CatchResult, Command};
use qualia::{Direction, InputCode, InputValue, InteractionMode, Key, KeyState};
use qualia::{Milliseconds, OptionalPosition, Position, Slide, Vector, modifier};
use qualia::{InputForwarding, InputHandling};
use qualia::{perceptron, Perceptron};

use config::KeybindingsConfig;
use binding_functions::{self, Executor};

// -------------------------------------------------------------------------------------------------

pub mod mode_name {
    pub const COMMON: &'static str = "common";
    pub const INSERT: &'static str = "insert";
    pub const NORMAL: &'static str = "normal";
    pub const VISUAL: &'static str = "visual";
}

// -------------------------------------------------------------------------------------------------

/// Structure representing mode.
pub struct Mode {
    active: bool,
    name: String,
    bindings: HashMap<Binding, Box<Executor>>,
    default_executor: Option<Box<Executor>>,
}

// -------------------------------------------------------------------------------------------------

impl Mode {
    /// Constructs new `Mode`.
    pub fn new(active: bool, name: String, default_executor: Option<Box<Executor>>) -> Self {
        Mode {
            active: active,
            name: name,
            bindings: HashMap::new(),
            default_executor: default_executor,
        }
    }

    /// Checks if mode is active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Returns name of the mode.
    pub fn get_name(&self) -> String {
        self.name.clone()
    }

    /// Sets mode active or inactive.
    pub fn make_active(&mut self, active: bool) {
        self.active = active;
    }

    /// Add new binding.
    pub fn add_binding(&mut self, binding: Binding, executor: Box<Executor>) {
        self.bindings.insert(binding, executor);
    }

    /// Returns executor for given binding.
    pub fn get_executor(&self, binding: &Binding) -> Option<&Box<Executor>> {
        let mut executor = self.bindings.get(binding);
        if self.default_executor.is_some() && executor.is_none() {
            executor = self.default_executor.as_ref();
        }
        executor
    }
}

// -------------------------------------------------------------------------------------------------

/// This structure manages key bindings and modes.
///
/// Main task is to identify key sequences as bindings and execute assigned function.
/// For thread-safe public version see `InputManager`.
struct InnerInputManager {
    modes: Vec<Mode>,
    code: InputCode,
    command: Command,
    previous_modification: binding_functions::PreviousModification,
    signaler: Signaler<Perceptron>,
}

// -------------------------------------------------------------------------------------------------

impl InnerInputManager {
    /// Constructs new `InnerInputManager`.
    pub fn new(config: &KeybindingsConfig, signaler: Signaler<Perceptron>) -> Self {
        // Create default modes
        let common_mode = Mode::new(true, mode_name::COMMON.to_owned(), None);
        let insert_mode = Mode::new(true, mode_name::INSERT.to_owned(), None);
        let normal_mode =
            Mode::new(false, mode_name::NORMAL.to_owned(), Some(binding_functions::Nop::new()));

        // Create manager
        let mut inner = InnerInputManager {
            modes: vec![common_mode, insert_mode, normal_mode],
            code: 0,
            command: Command::default(),
            previous_modification: binding_functions::PreviousModification::None,
            signaler: signaler,
        };

        inner.apply_configuration(config);
        inner
    }

    /// Applies the configuration.
    pub fn apply_configuration(&mut self, config: &KeybindingsConfig) {
        // Apply common mode bindings
        for b in config.common.iter() {
            self.add_binding(mode_name::COMMON.to_owned(),
                             b.binding.clone(),
                             b.executor.duplicate());
        }

        // Apply insert mode bindings
        for b in config.insert.iter() {
            self.add_binding(mode_name::INSERT.to_owned(),
                             b.binding.clone(),
                             b.executor.duplicate());
        }

        // Apply normal mode bindings
        for b in config.normal.iter() {
            self.add_binding(mode_name::NORMAL.to_owned(),
                             b.binding.clone(),
                             b.executor.duplicate());
        }

        // Apply visual mode bindings
        for b in config.visual.iter() {
            self.add_binding(mode_name::VISUAL.to_owned(),
                             b.binding.clone(),
                             b.executor.duplicate());
        }
    }

    /// Helper method for finding executor for given binding in active modes.
    fn find_executor(&self, binding: &Binding) -> Option<Box<Executor>> {
        for ref mode in self.modes.iter() {
            if mode.is_active() {
                if let Some(executor) = mode.get_executor(binding) {
                    return Some(executor.duplicate());
                }
            }
        }
        None
    }

    /// Tries for find executor matching to given key and state of modifiers and execute it if
    /// found.
    pub fn catch_key(&mut self,
                     code: InputCode,
                     value: InputValue,
                     modifiers: modifier::ModifierType)
                     -> CatchResult {
        self.catch(code, value, modifiers)
    }

    /// Tries for find executor matching to given button and state of modifiers and execute it if
    /// found.
    fn catch_button(&mut self,
                    code: InputCode,
                    value: InputValue,
                    modifiers: modifier::ModifierType)
                    -> CatchResult {
        self.catch(code, value, modifiers)
    }

    /// Helper method for searching and executing an executor.
    fn catch(&mut self,
             code: InputCode,
             value: InputValue,
             modifiers: modifier::ModifierType)
             -> CatchResult {
        self.code = code;
        if let Some(executor) = self.find_executor(&Binding::create(code, modifiers)) {
            if value == KeyState::Pressed as InputValue {
                executor.activate(self);
            } else if value == KeyState::Released as InputValue {
                executor.release(self);
            }
            CatchResult::Caught
        } else {
            CatchResult::Passed
        }
    }

    /// Activates or deactivates mode identified by name.
    pub fn make_mode_active(&mut self, mode_name: String, active: bool) {
        for ref mut mode in self.modes.iter_mut() {
            if mode.get_name() == mode_name {
                mode.make_active(active);
                break;
            }
        }
    }

    /// Adds given binding to mode identified by name.
    pub fn add_binding(&mut self, mode_name: String, binding: Binding, executor: Box<Executor>) {
        // Try to find mode and add binding to it
        let mut added = false;
        for ref mut mode in self.modes.iter_mut() {
            if mode.get_name() == mode_name {
                mode.add_binding(binding.clone(), executor.duplicate());
                added = true;
                break;
            }
        }

        // If mode not found - create new
        if !added {
            let mut mode = Mode::new(false, mode_name, None);
            mode.add_binding(binding, executor);
            self.modes.push(mode);
        }
    }
}

// -------------------------------------------------------------------------------------------------

// These methods will be called from executors when `InputManager` is locked so it is save to
// implement this trait for `InnerInputManager` instead of `InputManager`.
impl binding_functions::InputContext for InnerInputManager {
    fn set_action(&mut self, action: Action) {
        self.command.action = action;
        self.previous_modification = binding_functions::PreviousModification::Action;
    }

    fn set_direction(&mut self, direction: Direction) {
        self.command.direction = direction;
        self.previous_modification = binding_functions::PreviousModification::Direction;
    }

    fn set_magnitude(&mut self, magnitude: i32) {
        self.command.magnitude = magnitude;
        self.previous_modification = binding_functions::PreviousModification::Magnitude;
    }

    fn set_string(&mut self, string: String) {
        self.command.string = string;
        self.previous_modification = binding_functions::PreviousModification::String;
    }

    fn previous_modification(&self) -> binding_functions::PreviousModification {
        self.previous_modification
    }

    fn get_action(&mut self) -> Action {
        self.command.action
    }

    fn get_direction(&mut self) -> Direction {
        self.command.direction
    }

    fn get_magnitude(&mut self) -> i32 {
        self.command.magnitude
    }

    fn get_string(&mut self) -> String {
        self.command.string.clone()
    }

    fn execute_command(&mut self) {
        self.signaler.emit(perceptron::COMMAND, Perceptron::Command(self.command.clone()));
    }

    fn clean_command(&mut self) {
        self.command = Command::default();
        self.previous_modification = binding_functions::PreviousModification::None;
    }

    fn activate_mode(&mut self, mode_name: &'static str, active: bool) {
        self.make_mode_active(mode_name.to_string(), active);
        if mode_name == mode_name::VISUAL {
            self.signaler.emit(perceptron::MODE,
                               Perceptron::Mode {
                                   active: active,
                                   mode: InteractionMode::Visual,
                               });
        }
    }

    fn get_code(&self) -> InputCode {
        self.code
    }

    fn get_code_as_number(&self) -> Option<i32> {
        match self.code as i32 {
            uinput_sys::KEY_MINUS |
            uinput_sys::KEY_KPMINUS => Some(-1),
            uinput_sys::KEY_10 |
            uinput_sys::KEY_NUMERIC_0 => Some(0),
            uinput_sys::KEY_1 |
            uinput_sys::KEY_NUMERIC_1 => Some(1),
            uinput_sys::KEY_2 |
            uinput_sys::KEY_NUMERIC_2 => Some(2),
            uinput_sys::KEY_3 |
            uinput_sys::KEY_NUMERIC_3 => Some(3),
            uinput_sys::KEY_4 |
            uinput_sys::KEY_NUMERIC_4 => Some(4),
            uinput_sys::KEY_5 |
            uinput_sys::KEY_NUMERIC_5 => Some(5),
            uinput_sys::KEY_6 |
            uinput_sys::KEY_NUMERIC_6 => Some(6),
            uinput_sys::KEY_7 |
            uinput_sys::KEY_NUMERIC_7 => Some(7),
            uinput_sys::KEY_8 |
            uinput_sys::KEY_NUMERIC_8 => Some(8),
            uinput_sys::KEY_9 |
            uinput_sys::KEY_NUMERIC_9 => Some(9),
            _ => None,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Helper structure for locking `InnerInputManager` shared between threads.
///
/// Thread-safe public version of `InnerInputManager`.
#[derive(Clone)]
pub struct InputManager {
    inner: Arc<Mutex<InnerInputManager>>,
}

// -------------------------------------------------------------------------------------------------

impl InputManager {
    /// Constructs new `InputManager`.
    pub fn new(config: &KeybindingsConfig, signaler: Signaler<Perceptron>) -> Self {
        InputManager { inner: Arc::new(Mutex::new(InnerInputManager::new(config, signaler))) }
    }

    /// Lock and call corresponding method from `InnerInputManager`.
    pub fn make_mode_active(&mut self, mode_name: String, active: bool) {
        let mut mine = self.inner.lock().unwrap();
        mine.make_mode_active(mode_name, active)
    }

    /// Lock and call corresponding method from `InnerInputManager`.
    pub fn add_binding(&mut self, mode_name: String, binding: Binding, executor: Box<Executor>) {
        let mut mine = self.inner.lock().unwrap();
        mine.add_binding(mode_name, binding, executor)
    }
}

// -------------------------------------------------------------------------------------------------

impl InputHandling for InputManager {
    /// Lock and call corresponding method from `InnerInputManager`.
    fn catch_key(&mut self,
                 code: InputCode,
                 value: InputValue,
                 modifiers: modifier::ModifierType)
                 -> CatchResult {
        let mut mine = self.inner.lock().unwrap();
        mine.catch_key(code, value, modifiers)
    }

    /// Lock and call corresponding method from `InnerInputManager`.
    fn catch_button(&mut self,
                    code: InputCode,
                    value: InputValue,
                    modifiers: modifier::ModifierType)
                    -> CatchResult {
        let mut mine = self.inner.lock().unwrap();
        mine.catch_button(code, value, modifiers)
    }


    /// Clones the `InputManager` as unsized.
    fn duplicate(&self) -> Box<InputHandling> {
        Box::new(self.clone())
    }
}

// -------------------------------------------------------------------------------------------------

/// `InputForwarder` is used by input device drivers to notify the rest of the applications about
/// events.
#[derive(Clone)]
pub struct InputForwarder {
    /// Signaler for emitting signal to the rest of the application.
    signaler: Signaler<Perceptron>,

    /// Reference time for obtaining event timestamps.
    reference_time: Instant,
}

// -------------------------------------------------------------------------------------------------

impl InputForwarder {
    /// Constructs new `InputForwarder`.
    pub fn new(signaler: Signaler<Perceptron>, reference_time: Instant) -> Self {
        InputForwarder {
            signaler: signaler,
            reference_time: reference_time,
        }
    }

    /// Helper method for obtaining event time stamp as milliseconds.
    fn get_timestamp(&self) -> Milliseconds {
        Milliseconds::elapsed_from(&self.reference_time)
    }
}

// -------------------------------------------------------------------------------------------------

impl InputForwarding for InputForwarder {
    /// Emits key event.
    fn emit_key(&mut self, code: u16, value: i32) {
        let key = Key::new(code, value, self.get_timestamp());
        self.signaler.emit(perceptron::INPUT_KEYBOARD, Perceptron::InputKeyboard(key));
    }

    /// Emits pointer motion event.
    fn emit_motion(&mut self, x: isize, y: isize) {
        // Signal event
        self.signaler.emit(perceptron::INPUT_POINTER_MOTION,
                           Perceptron::InputPointerMotion(Vector::new(x, y)));
    }

    /// Emits pointer position event.
    fn emit_position(&mut self, x: Option<isize>, y: Option<isize>) {
        // Signal event
        self.signaler.emit(perceptron::INPUT_POINTER_POSITION,
                           Perceptron::InputPointerPosition(OptionalPosition::new(x, y)));
    }

    /// Emits button event.
    fn emit_button(&mut self, code: u16, value: i32) {
        let btn = Button::new(code, value, self.get_timestamp());

        // Signal event
        self.signaler.emit(perceptron::INPUT_POINTER_BUTTON, Perceptron::InputPointerButton(btn));
    }

    /// Emits exist event.
    fn emit_axis(&mut self, horizontal: isize, vertical: isize) {
        let axis = Axis::new(Vector::new(horizontal, vertical),
                             Slide::new(10.0 * horizontal as f32, 10.0 * vertical as f32),
                             self.get_timestamp());

        // Signal event
        self.signaler.emit(perceptron::INPUT_POINTER_AXIS, Perceptron::InputPointerAxis(axis));
    }

    /// Emits position reset event.
    fn emit_position_reset(&mut self, position: Option<Position>) {
        // Signal event
        self.signaler.emit(perceptron::INPUT_POINTER_POSITION_RESET,
                           Perceptron::InputPointerPositionReset(position));
    }

    /// Emits system activity event.
    fn emit_system_activity_event(&mut self) {
        self.signaler.emit(perceptron::NOTIFY, Perceptron::Notify);
    }
}

// -------------------------------------------------------------------------------------------------
