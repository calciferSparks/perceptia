// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of
// the MPL was not distributed with this file, You can obtain one at http://mozilla.org/MPL/2.0/

//! This module contains GL renderer which allows drawing frame scenes with GL.

// -------------------------------------------------------------------------------------------------

use std;
use std::time::Instant;

use gl;
use egl;

use cognitive_graphics::{egl_tools, gl_tools};
use cognitive_graphics::attributes::{DmabufAttributes, EglAttributes};
use qualia::{SurfaceViewer, SurfaceContext, Illusion, Size, PixelFormat, SurfaceId};
use qualia::{Buffer, DataSource, Image, MemoryView, Pixmap};

use cache_gl::CacheGl;

// -------------------------------------------------------------------------------------------------

/// Vertex shader source code for OpenGL ES 2.0 (GLSL ES 100)
const VERTEX_SHADER_100: &'static str = include_str!("vertex.100.glsl");

/// Fragment shader source code for OpenGL ES 2.0 (GLSL ES 100)
const FRAGMENT_SHADER_100: &'static str = include_str!("fragment.100.glsl");

/// Vertex shader source code for OpenGL ES 3.0 (GLSL ES 300)
const VERTEX_SHADER_300: &'static str = include_str!("vertex.300.glsl");

/// Fragment shader source code for OpenGL ES 3.0 (GLSL ES 300)
const FRAGMENT_SHADER_300: &'static str = include_str!("fragment.300.glsl");

// -------------------------------------------------------------------------------------------------

/// GL renderer.
pub struct RendererGl {
    egl: egl_tools::EglBucket,
    size: Size,
    cache: CacheGl,

    // GL rendering
    program: gl::types::GLuint,
    loc_vertices: gl::types::GLint,
    loc_texcoords: gl::types::GLint,
    loc_texture: gl::types::GLint,
    loc_screen_size: gl::types::GLint,
    vbo_vertices: gl::types::GLuint,
    vbo_texcoords: gl::types::GLuint,

    // Pointers to extension functions
    image_target_texture: Option<egl_tools::ImageTargetTexture2DOesFn>,
}

// -------------------------------------------------------------------------------------------------

impl RendererGl {
    /// `RendererGl` constructor.
    pub fn new(egl: egl_tools::EglBucket, size: Size) -> Self {
        RendererGl {
            egl: egl,
            size: size,
            cache: CacheGl::new(),
            program: gl::types::GLuint::default(),
            loc_vertices: gl::types::GLint::default(),
            loc_texcoords: gl::types::GLint::default(),
            loc_texture: gl::types::GLint::default(),
            loc_screen_size: gl::types::GLint::default(),
            vbo_vertices: gl::types::GLuint::default(),
            vbo_texcoords: gl::types::GLuint::default(),
            image_target_texture: None,
        }
    }

    /// Initialize renderer.
    ///  - prepare shaders and program,
    ///  - bind locations,
    ///  - generate buffers,
    ///  - configure textures,
    pub fn initialize(&mut self) -> Result<(), Illusion> {
        gl::load_with(|s| egl::get_proc_address(s) as *const std::os::raw::c_void);

        let _context = self.egl.make_current()?;

        // Get GLSL version
        let (vshader_src, fshader_src) = match gl_tools::get_shading_lang_version() {
            gl_tools::GlslVersion::Glsl100 => {
                (VERTEX_SHADER_100.to_owned(), FRAGMENT_SHADER_100.to_owned())
            }
            gl_tools::GlslVersion::Glsl300 => {
                (VERTEX_SHADER_300.to_owned(), FRAGMENT_SHADER_300.to_owned())
            }
            gl_tools::GlslVersion::Unknown => {
                return Err(Illusion::General(format!("Could not figure out GLSL version")));
            }
        };

        // Compile shades, link program and get locations
        self.program = gl_tools::prepare_shader_program(vshader_src, fshader_src)?;
        self.loc_vertices = gl_tools::get_attrib_location(self.program, "vertices".to_owned())?;
        self.loc_texcoords = gl_tools::get_attrib_location(self.program, "texcoords".to_owned())?;
        self.loc_texture = gl_tools::get_uniform_location(self.program, "texture".to_owned())?;
        self.loc_screen_size = gl_tools::get_uniform_location(self.program,
                                                              "screen_size".to_owned())?;

        // Generate vertex buffer object
        unsafe {
            gl::GenBuffers(1, &mut self.vbo_vertices);
            gl::GenBuffers(1, &mut self.vbo_texcoords);
        }

        // Get needed extension functions
        self.image_target_texture = egl_tools::get_proc_addr_of_image_target_texture_2d_oes();

        Ok(())
    }

    /// Draws passed frame scene.
    pub fn draw(&mut self,
                layunder: &Vec<SurfaceContext>,
                surfaces: &Vec<SurfaceContext>,
                layover: &Vec<SurfaceContext>,
                viewer: &SurfaceViewer)
                -> Result<(), Illusion> {
        let _context = self.egl.make_current()?;
        self.prepare_view();
        self.draw_surfaces(layunder, viewer);
        self.draw_surfaces(surfaces, viewer);
        self.draw_surfaces(layover, viewer);
        self.release_view();
        Ok(())
    }

    /// Swaps buffers.
    pub fn swap_buffers(&mut self) -> Result<(), Illusion> {
        let context = self.egl.make_current()?;
        context.swap_buffers()?;
        Ok(())
    }

    /// Reads pixels for whole screen and returns image data as `Buffer`.
    pub fn take_screenshot(&self) -> Result<Buffer, Illusion> {
        let _context = self.egl.make_current()?;

        let format = PixelFormat::ARGB8888;
        let stride = format.get_size() * self.size.width;
        let size = stride * self.size.height;
        let mut dst: Vec<u8> = Vec::with_capacity(size);
        unsafe { dst.set_len(size) };

        unsafe {
            gl::ReadBuffer(gl::BACK);
            gl::ReadPixels(0,
                           0,
                           self.size.width as i32,
                           self.size.height as i32,
                           gl::RGBA,
                           gl::UNSIGNED_BYTE,
                           dst.as_mut_ptr() as *mut std::os::raw::c_void);
        }

        // GL returns data starting from bottom. We have to reverse the order.
        let mut data = Vec::new();
        for chunk in dst.chunks(stride).rev() {
            data.extend(chunk);
        }

        Ok(Buffer::new(format, self.size.width, self.size.height, stride, data))
    }
}

// -------------------------------------------------------------------------------------------------

/// Drawing helpers.
impl RendererGl {
    /// Prepare view for drawing.
    fn prepare_view(&self) {
        unsafe {
            gl::ClearColor(0.0, 0.3, 0.5, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);

            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);

            gl::UseProgram(self.program);
            gl::Uniform2i(self.loc_screen_size, self.size.width as i32, self.size.height as i32);
        }
    }

    /// Loads memory buffer as texture. Returns dimensions of the buffer.
    fn load_buffer_as_texture(&mut self,
                              sid: SurfaceId,
                              buffer: &MemoryView,
                              time_stamp: Instant)
                              -> Option<Size> {
        let format = {
            match buffer.get_format() {
                // NOTE: Mixing channels is intentional. In `PixelFormat` one reads it from
                // right to left, and in `gl` from left to right.
                PixelFormat::XBGR8888 => gl::RGBA,
                PixelFormat::ABGR8888 => gl::RGBA,
                PixelFormat::XRGB8888 => gl::BGRA,
                PixelFormat::ARGB8888 => gl::BGRA,
            }
        };

        // Get or generate texture info
        let texinfo = self.cache.get_or_generate_info(sid);
        unsafe { gl::BindTexture(gl::TEXTURE_2D, texinfo.get_texture()) };

        // If buffer was updated recently - load it to GPU memory
        if texinfo.is_younger(time_stamp) {
            unsafe {
                gl::TexImage2D(gl::TEXTURE_2D, // target
                               0, // level, 0 = no mipmap
                               gl::RGBA as gl::types::GLint, // internal format
                               buffer.get_width() as gl::types::GLint, // width
                               buffer.get_height() as gl::types::GLint, // height
                               0, // always 0 in OpenGL ES
                               format, // format
                               gl::UNSIGNED_BYTE, // type
                               buffer.as_ptr() as *const _);
            }
            self.cache.update(sid, None);
        }

        Some(buffer.get_size())
    }

    /// Loads hardware image as texture. Returns dimensions of the image.
    fn load_image_as_texture(&mut self,
                             sid: SurfaceId,
                             attrs: &EglAttributes,
                             time_stamp: Instant)
                             -> Option<Size> {
        // Get or generate texture info
        let texinfo = self.cache.get_or_generate_info(sid);
        unsafe { gl::BindTexture(gl::TEXTURE_2D, texinfo.get_texture()) };

        // If buffer was updated recently - load it to GPU memory
        if texinfo.is_younger(time_stamp) {
            // Destroy image if it was created previously
            if let Some(image) = texinfo.get_image() {
                let _ = egl_tools::destroy_image(self.egl.display, image);
            }

            // Create the image
            if let Some(image_target_texture) = self.image_target_texture {
                let image = egl_tools::create_image(self.egl.display, attrs);
                if let Some(ref img) = image {
                    // Set image as texture target and update cache
                    image_target_texture(gl::TEXTURE_2D, img.as_raw());
                    self.cache.update(sid, image.clone());
                    Some(attrs.get_size())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            Some(attrs.get_size())
        }
    }

    /// Loads dmabuf as texture. Returns dimensions of the dmabuf.
    fn load_dmabuf_as_texture(&mut self,
                              sid: SurfaceId,
                              attrs: &DmabufAttributes,
                              time_stamp: Instant)
                              -> Option<Size> {
        // Get or generate texture info
        let texinfo = self.cache.get_or_generate_info(sid);
        unsafe { gl::BindTexture(gl::TEXTURE_2D, texinfo.get_texture()) };

        // If buffer was updated recently - load it to GPU memory
        if texinfo.is_younger(time_stamp) {
            // Destroy image if it was created previously
            if let Some(image) = texinfo.get_image() {
                let _ = egl_tools::destroy_image(self.egl.display, image);
            }

            // Create the image
            if let Some(image_target_texture) = self.image_target_texture {
                let image = egl_tools::import_dmabuf(self.egl.display, attrs);
                if let Some(ref img) = image {
                    // Set image as texture target and update cache
                    image_target_texture(gl::TEXTURE_2D, img.as_raw());
                    self.cache.update(sid, image.clone());
                    Some(attrs.get_size())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            Some(attrs.get_size())
        }
    }

    /// Load textures and prepare vertices.
    fn load_texture_and_prepare_vertices(&mut self,
                                         viewer: &SurfaceViewer,
                                         context: &SurfaceContext,
                                         vertices: &mut [gl::types::GLfloat],
                                         texcoords: &mut [gl::types::GLfloat]) {
        if let Some(ref surface) = viewer.get_surface(context.id) {
            let size = {
                match surface.data_source {
                    DataSource::Shm { ref source, time_stamp } => {
                        self.load_buffer_as_texture(context.id, source, time_stamp)
                    }
                    DataSource::EglImage { ref source, time_stamp } => {
                        self.load_image_as_texture(context.id, source, time_stamp)
                    }
                    DataSource::Dmabuf { ref source, time_stamp } => {
                        self.load_dmabuf_as_texture(context.id, source, time_stamp)
                    }
                    DataSource::None => None,
                }
            };

            if let Some(size) = size {
                let left = (context.pos.x - surface.offset.x) as gl::types::GLfloat;
                let top = (context.pos.y - surface.offset.y) as gl::types::GLfloat;
                let right = left + size.width as gl::types::GLfloat;
                let bottom = top + size.height as gl::types::GLfloat;

                vertices[0] = left;
                vertices[1] = top;
                vertices[2] = right;
                vertices[3] = top;
                vertices[4] = left;
                vertices[5] = bottom;
                vertices[6] = right;
                vertices[7] = top;
                vertices[8] = right;
                vertices[9] = bottom;
                vertices[10] = left;
                vertices[11] = bottom;

                // TODO: Use element buffer.
                texcoords[0] = 0.0;
                texcoords[1] = 0.0;
                texcoords[2] = 1.0;
                texcoords[3] = 0.0;
                texcoords[4] = 0.0;
                texcoords[5] = 1.0;
                texcoords[6] = 1.0;
                texcoords[7] = 0.0;
                texcoords[8] = 1.0;
                texcoords[9] = 1.0;
                texcoords[10] = 0.0;
                texcoords[11] = 1.0;
            } else {
                log_warn3!("Renderer: No buffer for surface {}", context.id);
            }
        } else {
            log_warn3!("Renderer: No info for surface {}", context.id);
        }
    }

    /// Draws surfaces.
    fn draw_surfaces(&mut self, surfaces: &Vec<SurfaceContext>, viewer: &SurfaceViewer) {
        if surfaces.len() == 0 {
            return;
        }

        // Prepare vertices positions and upload textures
        let vertices_len = 12 * surfaces.len();
        let vertices_size = vertices_len * std::mem::size_of::<gl::types::GLfloat>();
        let mut vertices = vec![0.0; vertices_len];
        let mut texcoords = vec![0.0; vertices_len];

        for i in 0..surfaces.len() {
            // Activate the target texture
            unsafe { gl::ActiveTexture(gl::TEXTURE0 + i as u32) };

            // Bind data to texture and prepare vertices
            self.load_texture_and_prepare_vertices(viewer,
                                                   &surfaces[i],
                                                   &mut vertices[12 * i..12 * i + 12],
                                                   &mut texcoords[12 * i..12 * i + 12]);
        }

        unsafe {
            // Upload positions to vertex buffer object
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo_vertices);
            gl::EnableVertexAttribArray(self.loc_vertices as gl::types::GLuint);
            gl::VertexAttribPointer(self.loc_vertices as gl::types::GLuint,
                                    2,
                                    gl::FLOAT,
                                    gl::FALSE,
                                    2 *
                                    std::mem::size_of::<gl::types::GLfloat>() as gl::types::GLint,
                                    std::ptr::null());
            gl::BufferData(gl::ARRAY_BUFFER,
                           vertices_size as isize,
                           vertices.as_ptr() as *const _,
                           gl::DYNAMIC_DRAW);

            // Upload positions to vertex buffer object
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo_texcoords);
            gl::EnableVertexAttribArray(self.loc_texcoords as gl::types::GLuint);
            gl::VertexAttribPointer(self.loc_texcoords as gl::types::GLuint,
                                    2,
                                    gl::FLOAT,
                                    gl::FALSE,
                                    2 *
                                    std::mem::size_of::<gl::types::GLfloat>() as gl::types::GLint,
                                    std::ptr::null());
            gl::BufferData(gl::ARRAY_BUFFER,
                           vertices_size as isize,
                           texcoords.as_ptr() as *const _,
                           gl::DYNAMIC_DRAW);

            // Redraw everything
            for i in 0..surfaces.len() as i32 {
                gl::Uniform1i(self.loc_texture, i);
                gl::DrawArrays(gl::TRIANGLES, 6 * i, 6);
            }

            // Release resources
            gl::DisableVertexAttribArray(self.loc_texcoords as gl::types::GLuint);
            gl::DisableVertexAttribArray(self.loc_vertices as gl::types::GLuint);
        }
    }

    /// Unbind framebuffer and program.
    fn release_view(&self) {
        unsafe {
            gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, 0);
            gl::UseProgram(0);
        }
    }
}

// -------------------------------------------------------------------------------------------------
