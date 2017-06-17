// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of
// the MPL was not distributed with this file, You can obtain one at http://mozilla.org/MPL/2.0/

//! This crate contains code dedicated to managing output device like buffer swapping or controlling
//! v-blanks.

extern crate libc;
extern crate egl;
extern crate drm as libdrm;
extern crate gbm_rs as libgbm;

extern crate cognitive_graphics as graphics;
extern crate cognitive_qualia as qualia;
extern crate cognitive_renderer_gl as renderer_gl;
extern crate cognitive_renderer_pixmap as renderer_pixmap;

mod output;
pub use output::Output;

mod drm_output;
pub use drm_output::DrmOutput;

mod virtual_output;
pub use virtual_output::VirtualOutput;

#[cfg(feature = "testing")]
pub mod output_mock;
