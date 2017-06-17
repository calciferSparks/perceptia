// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of
// the MPL was not distributed with this file, You can obtain one at http://mozilla.org/MPL/2.0/

//! This module contains common state between handlers belonging to one client.

// -------------------------------------------------------------------------------------------------

use std;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::os::unix::io::RawFd;

use dharma;
use skylane::server as wl;
use skylane_protocols::server::wayland::{wl_display, wl_callback, wl_buffer};
use skylane_protocols::server::wayland::{wl_keyboard, wl_pointer};
use skylane_protocols::server::wayland::{wl_data_source, wl_data_device, wl_data_offer};
use skylane_protocols::server::wayland::wl_shell_surface;
use skylane_protocols::server::xdg_shell_unstable_v6::{zxdg_toplevel_v6, zxdg_surface_v6};
use skylane_protocols::server::weston_screenshooter::weston_screenshooter;

use cognitive_graphics::attributes::{EglAttributes, DmabufAttributes};
use qualia::{Settings, Transfer, OutputType, Memory};
use qualia::{Area, Axis, Button, Key, Milliseconds};
use qualia::{OutputInfo, PixelFormat, Position, Size, Vector};
use qualia::{DmabufId, EglImageId, MemoryPoolId, MemoryViewId};
use qualia::{show_reason, surface_state, SurfaceId};
use qualia::FrontendsCoordinationTrait;
use inputs::KeyMods;

use protocol;
use facade::{Facade, PositionerInfo, ShellSurfaceOid};
use gateway::Gateway;
use global::Global;
use mediator::MediatorRef;

// -------------------------------------------------------------------------------------------------

/// Helper macro for relating surface information with surface ID
macro_rules! relate_sid_with {
    ($member:ident, $dict:expr, $sid:ident, $obj:ident) => {
        if {
            if let Some(info) = $dict.get_mut(&$sid) {
                if info.$member.is_none() {
                    info.$member = Some($obj);
                } else {
                    log_warn3!("Already have {} for surface {}", stringify!($member), $sid);
                }
                false
            } else {
                true
            }
        } {
            let mut info = SurfaceInfo::new();
            info.$member = Some($obj);
            $dict.insert($sid, info);
        }
    }
}

// -------------------------------------------------------------------------------------------------

macro_rules! unrelate_sid_with {
    ($member:ident, $dict:expr, $sid:ident) => {
        if let Some(info) = $dict.get_mut(&$sid) {
            info.$member = None;
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Helper structure for aggregating information about surface.
#[derive(Debug)]
struct SurfaceInfo {
    // For sending keyboard `enter` and `leave`.
    surface_oid: Option<wl::ObjectId>,

    // For releasing buffer in `on_surface_frame`
    buffer_oid: Option<wl::ObjectId>,

    // For sending frame callback in `on_surface_frame`
    frame_oid: Option<wl::ObjectId>,

    // For send reconfiguration events in `on_surface_reconfigured`
    shell_surface_oid: Option<ShellSurfaceOid>,
}

// -------------------------------------------------------------------------------------------------

impl SurfaceInfo {
    pub fn new() -> Self {
        SurfaceInfo {
            surface_oid: None,
            shell_surface_oid: None,
            buffer_oid: None,
            frame_oid: None,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Helper structure for aggregating information about buffers.
#[derive(Clone)]
enum BufferInfo {
    Shm {
        mpid: MemoryPoolId,
        mvid: MemoryViewId,
    },
    EglImage { eiid: EglImageId },
    Dmabuf { dbid: DmabufId },
}

// -------------------------------------------------------------------------------------------------

impl BufferInfo {
    pub fn new_shm(mpid: MemoryPoolId, mvid: MemoryViewId) -> Self {
        BufferInfo::Shm {
            mpid: mpid,
            mvid: mvid,
        }
    }

    pub fn new_egl_image(eiid: EglImageId) -> Self {
        BufferInfo::EglImage { eiid: eiid }
    }

    pub fn new_dmabuf(dbid: DmabufId) -> Self {
        BufferInfo::Dmabuf { dbid: dbid }
    }
}

// -------------------------------------------------------------------------------------------------

/// `Proxy` holds information common between handlers of one client. It constitutes for them facade
/// for rest of the crate/application and gateway from `Engine` to clients.
///
/// For information about its place among other structures see crate-level documentation.
///
/// TODO: Add more information about members of `Proxy`.
pub struct Proxy {
    client_id: dharma::EventHandlerId,
    coordinator: Box<FrontendsCoordinationTrait>,
    settings: Settings,

    mediator: MediatorRef,
    socket: wl::Socket,

    /// Map from global name to global info structure.
    ///
    /// NOTE: It must be possible to iterate globals in order of registering because advertising
    /// globals in wrong order may crash clients
    globals: BTreeMap<u32, Global>,

    regions: HashMap<wl::ObjectId, Area>,
    positioners: HashMap<wl::ObjectId, PositionerInfo>,
    transfers: HashMap<wl::ObjectId, Transfer>,
    pointer_oids: HashSet<wl::ObjectId>,
    keyboard_oids: HashSet<wl::ObjectId>,
    data_device_oids: HashSet<wl::ObjectId>,
    data_source_oid: Option<wl::ObjectId>,
    memory_pools: HashSet<MemoryPoolId>,
    surface_oid_to_sid_dict: HashMap<wl::ObjectId, SurfaceId>,
    sid_to_surface_info_dict: HashMap<SurfaceId, SurfaceInfo>,

    /// Surface object does not know if its buffer is memory shared or hardware image. It can only
    /// tell its buffer object ID, so we map this ID to structure defining how it can be attached.
    buffer_oid_to_info_dict: HashMap<wl::ObjectId, BufferInfo>,
    output_oid_to_id: HashMap<wl::ObjectId, i32>,
    screenshooter_oid: Option<wl::ObjectId>,
    screenshot_memory: Option<Memory>,
    last_global_id: u32,
}

define_ref!(struct Proxy as ProxyRef);

// -------------------------------------------------------------------------------------------------

impl Proxy {
    /// Creates new `Proxy`.
    pub fn new(client_id: dharma::EventHandlerId,
               coordinator: Box<FrontendsCoordinationTrait>,
               settings: Settings,
               mediator: MediatorRef,
               socket: wl::Socket)
               -> Self {
        Proxy {
            client_id: client_id,
            coordinator: coordinator,
            mediator: mediator,
            settings: settings,
            socket: socket,
            globals: BTreeMap::new(),
            regions: HashMap::new(),
            positioners: HashMap::new(),
            transfers: HashMap::new(),
            pointer_oids: HashSet::new(),
            keyboard_oids: HashSet::new(),
            data_device_oids: HashSet::new(),
            data_source_oid: None,
            memory_pools: HashSet::new(),
            surface_oid_to_sid_dict: HashMap::new(),
            sid_to_surface_info_dict: HashMap::new(),
            buffer_oid_to_info_dict: HashMap::new(),
            output_oid_to_id: HashMap::new(),
            screenshooter_oid: None,
            screenshot_memory: None,
            last_global_id: 0,
        }
    }

    /// Returns copy of application settings.
    pub fn get_settings(&self) -> Settings {
        self.settings.clone()
    }

    /// Returns client connection socket.
    pub fn get_socket(&self) -> wl::Socket {
        self.socket.clone()
    }

    /// Return list of current globals.
    pub fn get_globals(&self) -> &BTreeMap<u32, Global> {
        &self.globals
    }

    /// Registers new global.
    pub fn register_global(&mut self, mut global: Global) {
        self.last_global_id += 1;
        global.name = self.last_global_id;
        self.globals.insert(self.last_global_id, global);
    }

    /// Handles termination of client by destroying its resources.
    pub fn terminate(&mut self) {
        for mpid in self.memory_pools.iter() {
            self.coordinator.destroy_memory_pool(*mpid);
        }

        for (_, sid) in self.surface_oid_to_sid_dict.iter() {
            self.mediator.borrow_mut().remove(*sid);
            self.coordinator.destroy_surface(*sid);
        }
    }
}

// -------------------------------------------------------------------------------------------------

impl Proxy {
    /// Helper method for setting surface information for surface.
    fn relate_sid_with_surface(&mut self, sid: SurfaceId, oid: wl::ObjectId) {
        self.surface_oid_to_sid_dict.insert(oid, sid);
        relate_sid_with!(surface_oid, self.sid_to_surface_info_dict, sid, oid);
    }

    /// Helper method for setting shell information for surface.
    fn relate_sid_with_shell_surface(&mut self, sid: SurfaceId, oid: ShellSurfaceOid) {
        relate_sid_with!(shell_surface_oid, self.sid_to_surface_info_dict, sid, oid);
    }

    /// Helper method for setting buffer information for surface.
    fn relate_sid_with_buffer(&mut self, sid: SurfaceId, oid: wl::ObjectId) {
        relate_sid_with!(buffer_oid, self.sid_to_surface_info_dict, sid, oid);
    }

    /// Helper method for setting frame callback ID information for surface.
    fn relate_sid_with_frame(&mut self, sid: SurfaceId, oid: wl::ObjectId) {
        relate_sid_with!(frame_oid, self.sid_to_surface_info_dict, sid, oid);
    }
}

// -------------------------------------------------------------------------------------------------

impl Proxy {
    /// Helper method for unsetting shell information for surface.
    fn unrelate_sid_with_shell_surface(&mut self, sid: SurfaceId) {
        unrelate_sid_with!(shell_surface_oid, self.sid_to_surface_info_dict, sid);
    }
}

// -------------------------------------------------------------------------------------------------

// Other functions (which should be probably refactored).
impl Proxy {
    pub fn get_surface_oid_for_shell(&self,
                                     parent_shell_surface_oid: wl::ObjectId)
                                     -> Option<wl::ObjectId> {
        for info in self.sid_to_surface_info_dict.values() {
            if let Some(shell_surface_oid) = info.shell_surface_oid {
                match shell_surface_oid {
                    ShellSurfaceOid::ZxdgToplevelV6(shell_surface_oid, _) => {
                        if shell_surface_oid == parent_shell_surface_oid {
                            return info.surface_oid;
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    pub fn make_data_offer(&mut self, connection: &mut wl::Connection, proxy_ref: ProxyRef) {
        if let Some(transfer) = self.coordinator.get_transfer() {
            for data_device_oid in self.data_device_oids.iter() {
                let data_offer_oid = connection.get_next_available_server_object_id();
                let data_offer =
                    protocol::data_device_manager::DataOffer::new_object(data_offer_oid,
                                                                         proxy_ref.clone());
                connection.add_object(data_offer_oid, data_offer);

                send!(wl_data_device::data_offer(&self.socket, *data_device_oid, data_offer_oid));

                for mime_type in transfer.get_mime_types() {
                    send!(wl_data_offer::offer(&self.socket, data_offer_oid, mime_type));
                }

                send!(wl_data_device::selection(&self.socket, *data_device_oid, data_offer_oid));
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

#[allow(unused_variables)]
impl Facade for Proxy {
    fn create_memory_pool(&mut self, memory: Memory) -> MemoryPoolId {
        let mpid = self.coordinator.create_memory_pool(memory);
        self.memory_pools.insert(mpid);
        mpid
    }

    fn destroy_memory_pool(&mut self, mpid: MemoryPoolId) {
        self.memory_pools.remove(&mpid);
        self.coordinator.destroy_memory_pool(mpid);
    }

    fn replace_memory_pool(&mut self, mpid: MemoryPoolId, memory: Memory) {
        self.coordinator.replace_memory_pool(mpid, memory);
    }

    fn create_memory_view(&mut self,
                          mpid: MemoryPoolId,
                          buffer_oid: wl::ObjectId,
                          format: PixelFormat,
                          offset: usize,
                          width: usize,
                          height: usize,
                          stride: usize)
                          -> Option<MemoryViewId> {
        let result =
            self.coordinator.create_memory_view(mpid, format, offset, width, height, stride);
        if let Some(mvid) = result {
            let info = BufferInfo::new_shm(mpid, mvid);
            self.buffer_oid_to_info_dict.insert(buffer_oid, info);
        }
        result
    }

    fn destroy_memory_view(&mut self, mvid: MemoryViewId) {
        self.coordinator.destroy_memory_view(mvid);
    }

    fn create_egl_image(&mut self,
                        buffer_oid: wl::ObjectId,
                        attrs: EglAttributes)
                        -> Option<EglImageId> {
        let eiid = self.coordinator.create_egl_image(attrs);
        if let Some(eiid) = eiid {
            let info = BufferInfo::new_egl_image(eiid);
            self.buffer_oid_to_info_dict.insert(buffer_oid, info);
        }
        eiid
    }

    fn destroy_egl_image(&mut self, eiid: EglImageId) {
        self.coordinator.destroy_egl_image(eiid);
    }

    fn import_dmabuf(&mut self,
                     buffer_oid: wl::ObjectId,
                     attrs: DmabufAttributes)
                     -> Option<DmabufId> {
        let dbid = self.coordinator.import_dmabuf(attrs);
        if let Some(dbid) = dbid {
            let info = BufferInfo::new_dmabuf(dbid);
            self.buffer_oid_to_info_dict.insert(buffer_oid, info);
        }
        dbid
    }

    fn destroy_dmabuf(&mut self, dbid: DmabufId) {
        self.coordinator.destroy_dmabuf(dbid);
    }

    fn define_region(&mut self, region_oid: wl::ObjectId, region: Area) {
        self.regions.insert(region_oid, region);
    }

    fn undefine_region(&mut self, region_oid: wl::ObjectId) {
        self.regions.remove(&region_oid);
    }

    fn add_pointer_oid(&mut self, pointer_oid: wl::ObjectId) {
        self.pointer_oids.insert(pointer_oid);
    }

    fn remove_pointer_oid(&mut self, pointer_oid: wl::ObjectId) {
        self.pointer_oids.remove(&pointer_oid);
    }

    fn add_keyboard_oid(&mut self, keyboard_oid: wl::ObjectId) {
        self.keyboard_oids.insert(keyboard_oid);
    }

    fn remove_keyboard_oid(&mut self, keyboard_oid: wl::ObjectId) {
        self.keyboard_oids.remove(&keyboard_oid);
    }

    fn add_data_device_oid(&mut self, data_device_oid: wl::ObjectId) {
        self.data_device_oids.insert(data_device_oid);
    }

    fn remove_data_device_oid(&mut self, data_device_oid: wl::ObjectId) {
        self.data_device_oids.remove(&data_device_oid);
    }

    fn set_positioner(&mut self, oid: wl::ObjectId, positioner: PositionerInfo) {
        self.positioners.insert(oid, positioner);
    }

    fn get_positioner(&mut self, oid: wl::ObjectId) -> Option<PositionerInfo> {
        if let Some(positioner) = self.positioners.get(&oid) {
            Some(*positioner)
        } else {
            None
        }
    }

    fn remove_positioner(&mut self, oid: wl::ObjectId) {
        self.positioners.remove(&oid);
    }

    fn set_transfer(&mut self, oid: wl::ObjectId, transfer: Transfer) {
        self.transfers.insert(oid, transfer);
    }

    fn get_transfer(&mut self, oid: wl::ObjectId) -> Option<Transfer> {
        if let Some(transfer) = self.transfers.get(&oid) {
            Some(transfer.clone())
        } else {
            None
        }
    }

    fn select_transfer(&mut self, data_source_oid: wl::ObjectId) {
        if let Some(transfer) = self.transfers.get(&data_source_oid) {
            self.data_source_oid = Some(data_source_oid);
            self.mediator.borrow_mut().register_transfer_offerer(Some(self.client_id));
            self.coordinator.set_transfer(Some(transfer.clone()));
        } else {
            log_warn2!("Transfer cannot be selected");
        }
    }

    fn remove_transfer(&mut self, oid: wl::ObjectId) {
        self.transfers.remove(&oid);
    }

    fn request_transfer(&mut self, mime_type: String, fd: RawFd) {
        self.coordinator.request_transfer(mime_type, fd);
    }

    fn set_input_region(&self, sid: SurfaceId, region_oid: wl::ObjectId) {
        if let Some(region) = self.regions.get(&region_oid) {
            self.coordinator.set_surface_offset(sid, region.pos);
            self.coordinator.set_surface_requested_size(sid, region.size);
        } else {
            // TODO: Implement reseting input region.
        }
    }

    fn create_surface(&mut self, oid: wl::ObjectId) -> SurfaceId {
        let sid = self.coordinator.create_surface();
        self.relate_sid_with_surface(sid, oid);
        self.mediator.borrow_mut().relate_sid_to_client(sid, self.client_id);
        sid
    }

    fn destroy_surface(&self, sid: SurfaceId) {
        self.coordinator.destroy_surface(sid)
    }

    fn attach(&mut self, buffer_oid: wl::ObjectId, sid: SurfaceId, _x: i32, _y: i32) {
        if buffer_oid.is_null() {
            // Client wants to unmap this surface
            // TODO: This should be done on commit
            self.coordinator.unrelate_surface(sid);
            self.coordinator.detach_surface(sid)
        } else if let Some(info) = self.buffer_oid_to_info_dict.get(&buffer_oid).cloned() {
            self.relate_sid_with_buffer(sid, buffer_oid);
            match info {
                BufferInfo::Shm { mpid, mvid } => {
                    self.coordinator.attach_shm(mvid, sid);
                }
                BufferInfo::EglImage { eiid } => {
                    self.coordinator.attach_egl_image(eiid, sid);
                }
                BufferInfo::Dmabuf { dbid } => {
                    self.coordinator.attach_dmabuf(dbid, sid);
                }
            }
        } else {
            log_error!("Unknown buffer object ID: {}", buffer_oid);
        }
    }

    fn commit(&self, sid: SurfaceId) {
        self.coordinator.commit_surface(sid);
    }

    fn set_frame(&mut self, sid: SurfaceId, frame_oid: wl::ObjectId) {
        self.relate_sid_with_frame(sid, frame_oid);
    }

    fn show(&mut self,
            surface_oid: wl::ObjectId,
            shell_surface_oid: ShellSurfaceOid,
            reason: show_reason::ShowReason) {
        if let Some(&sid) = self.surface_oid_to_sid_dict.get(&surface_oid) {
            self.relate_sid_with_shell_surface(sid, shell_surface_oid);
            self.coordinator.show_surface(sid, reason);
        } else {
            log_error!("Unknown surface object ID: {}", surface_oid);
        }
    }

    fn hide(&mut self, surface_oid: wl::ObjectId, reason: show_reason::ShowReason) {
        if let Some(&sid) = self.surface_oid_to_sid_dict.get(&surface_oid) {
            self.coordinator.hide_surface(sid, reason);
            self.unrelate_sid_with_shell_surface(sid);
        } else {
            log_error!("Unknown surface object ID: {}", surface_oid);
        }
    }

    fn set_offset(&self, sid: SurfaceId, offset: Vector) {
        self.coordinator.set_surface_offset(sid, offset);
    }

    fn set_requested_size(&self, sid: SurfaceId, size: Size) {
        self.coordinator.set_surface_requested_size(sid, size);
    }

    fn set_relative_position(&self, surface_oid: wl::ObjectId, x: isize, y: isize) {
        if let Some(&sid) = self.surface_oid_to_sid_dict.get(&surface_oid) {
            let position = Position::new(x, y);
            self.coordinator.set_surface_relative_position(sid, position);
        }
    }

    fn relate(&self, surface_oid: wl::ObjectId, parent_surface_oid: wl::ObjectId) {
        if let Some(&sid) = self.surface_oid_to_sid_dict.get(&surface_oid) {
            if let Some(&parent_sid) = self.surface_oid_to_sid_dict.get(&parent_surface_oid) {
                self.coordinator.relate_surfaces(sid, parent_sid);
            }
            self.coordinator.set_surface_relative_position(sid, Position::default());
        }
    }

    fn unrelate(&self, surface_oid: wl::ObjectId) {
        if let Some(&sid) = self.surface_oid_to_sid_dict.get(&surface_oid) {
            self.coordinator.unrelate_surface(sid);
        }
    }

    fn set_as_cursor(&self, surface_oid: wl::ObjectId, hotspot_x: isize, hotspot_y: isize) {
        if let Some(&sid) = self.surface_oid_to_sid_dict.get(&surface_oid) {
            let position = Position::new(hotspot_x, hotspot_y);
            self.coordinator.set_surface_offset(sid, position);
            self.coordinator.set_surface_as_cursor(sid);
        }
    }

    fn relate_output_oid_with_id(&mut self, oid: wl::ObjectId, id: i32) {
        self.output_oid_to_id.insert(oid, id);
    }

    fn take_screenshot(&mut self,
                       screenshooter_oid: wl::ObjectId,
                       output_oid: wl::ObjectId,
                       buffer_oid: wl::ObjectId) {
        // Destroy memory pool to be used to transfer screenshot.
        if let Some(&BufferInfo::Shm { mpid, mvid }) =
            self.buffer_oid_to_info_dict.get(&buffer_oid) {
            self.screenshot_memory = self.coordinator.destroy_memory_pool(mpid);
        }

        // If the memory was not in use the mapped memory will be returned.
        if self.screenshot_memory.is_some() {
            if let Some(output_id) = self.output_oid_to_id.get(&output_oid) {
                // Request to take screenshot asynchronously. After data is ready method
                // `on_screenshot_done` will be called.
                self.coordinator.take_screenshot(*output_id);

                // Save ID of client requesting screenshot for later use.
                self.mediator.borrow_mut().register_screenshoter(Some(self.client_id));

                // Save screenshooter object ID for later use.
                self.screenshooter_oid = Some(screenshooter_oid);
            } else {
                log_warn1!("No matching output for screenshot");
            }
        } else {
            log_warn1!("Could not set buffer up for screenshot");
        }
    }

    fn authenticate_drm_device(&mut self, magic: u32) {
        self.mediator.borrow().authenticate_drm_device(magic);
    }

    fn get_drm_device_path(&self) -> Option<PathBuf> {
        self.mediator.borrow().get_drm_device_path()
    }
}

// -------------------------------------------------------------------------------------------------

impl Gateway for Proxy {
    fn on_output_found(&mut self, _output_type: OutputType) {}

    fn on_display_created(&mut self, output_info: OutputInfo) {
        self.register_global(protocol::output::get_global(output_info));
    }

    fn on_keyboard_input(&mut self, key: Key, mods: Option<KeyMods>) {
        for &keyboard_oid in self.keyboard_oids.iter() {
            let mut serial = self.socket.get_next_serial();
            send!(wl_keyboard::key(&self.socket,
                                   keyboard_oid,
                                   serial,
                                   key.time.get_value() as u32,
                                   key.code as u32,
                                   key.value as u32));

            if let Some(mods) = mods {
                serial = self.socket.get_next_serial();
                send!(wl_keyboard::modifiers(&self.socket,
                                             keyboard_oid,
                                             serial,
                                             mods.depressed,
                                             mods.latched,
                                             mods.locked,
                                             mods.effective));
            }
        }
    }

    fn on_surface_frame(&mut self, sid: SurfaceId, milliseconds: Milliseconds) {
        if let Some(info) = self.sid_to_surface_info_dict.get_mut(&sid) {
            if let Some(frame_oid) = info.frame_oid {
                send!(wl_callback::done(&self.socket, frame_oid, milliseconds.get_value() as u32));
                send!(wl_display::delete_id(&self.socket, wl::DISPLAY_ID, frame_oid.get_value()));
            }
            info.frame_oid = None;

            if let Some(buffer_oid) = info.buffer_oid {
                send!(wl_buffer::release(&self.socket, buffer_oid));
            }
            info.buffer_oid = None;
        }
    }

    fn on_pointer_focus_changed(&self,
                                old_sid: SurfaceId,
                                new_sid: SurfaceId,
                                position: Position) {
        if old_sid != SurfaceId::invalid() {
            if let Some(surface_info) = self.sid_to_surface_info_dict.get(&old_sid) {
                if let Some(surface_oid) = surface_info.surface_oid {
                    for pointer_oid in self.pointer_oids.iter() {
                        let serial = self.socket.get_next_serial();
                        send!(wl_pointer::leave(&self.socket, *pointer_oid, serial, surface_oid));
                    }
                }
            }
        }

        if new_sid != SurfaceId::invalid() {
            if let Some(surface_info) = self.sid_to_surface_info_dict.get(&new_sid) {
                if let Some(surface_oid) = surface_info.surface_oid {
                    for pointer_oid in self.pointer_oids.iter() {
                        let serial = self.socket.get_next_serial();
                        send!(wl_pointer::enter(&self.socket,
                                                *pointer_oid,
                                                serial,
                                                surface_oid,
                                                position.x as f32,
                                                position.y as f32));
                    }
                }
            }
        }
    }

    fn on_pointer_relative_motion(&self,
                                  _sid: SurfaceId,
                                  position: Position,
                                  milliseconds: Milliseconds) {
        for pointer_oid in self.pointer_oids.iter() {
            send!(wl_pointer::motion(&self.socket,
                                     *pointer_oid,
                                     milliseconds.get_value() as u32,
                                     position.x as f32,
                                     position.y as f32));
        }
    }

    fn on_pointer_button(&self, btn: Button) {
        let serial = self.socket.get_next_serial();
        let state = if btn.value == 0 {
            wl_pointer::button_state::RELEASED
        } else {
            wl_pointer::button_state::PRESSED
        };

        for pointer_oid in self.pointer_oids.iter() {
            send!(wl_pointer::button(&self.socket,
                                     *pointer_oid,
                                     serial,
                                     btn.time.get_value() as u32,
                                     btn.code as u32,
                                     state));
        }
    }

    fn on_pointer_axis(&self, mut axis: Axis) {
        axis.discrete.y = -1 * axis.discrete.y;
        axis.continuous.y = -1.0 * axis.continuous.y;

        for pointer_oid in self.pointer_oids.iter() {
            // vertical scroll
            let axis_type = wl_pointer::axis::VERTICAL_SCROLL;

            if axis.discrete.y != 0 {
                send!(wl_pointer::axis_discrete(&self.socket,
                                                *pointer_oid,
                                                axis_type,
                                                axis.discrete.y as i32));
            }

            if axis.continuous.y != 0.0 {
                send!(wl_pointer::axis(&self.socket,
                                       *pointer_oid,
                                       axis.time.get_value() as u32,
                                       axis_type,
                                       axis.continuous.y));
            } else {
                send!(wl_pointer::axis_stop(&self.socket,
                                            *pointer_oid,
                                            axis.time.get_value() as u32,
                                            axis_type));
            }

            // horizontal scroll
            let axis_type = wl_pointer::axis::HORIZONTAL_SCROLL;

            if axis.discrete.x != 0 {
                send!(wl_pointer::axis_discrete(&self.socket,
                                                *pointer_oid,
                                                axis_type,
                                                axis.discrete.x as i32));
            }

            if axis.continuous.x != 0.0 {
                send!(wl_pointer::axis(&self.socket,
                                       *pointer_oid,
                                       axis.time.get_value() as u32,
                                       axis_type,
                                       axis.continuous.x));
            } else {
                send!(wl_pointer::axis_stop(&self.socket,
                                            *pointer_oid,
                                            axis.time.get_value() as u32,
                                            axis_type));
            }

            // send frame
            send!(wl_pointer::frame(&self.socket, *pointer_oid));
        }
    }

    fn on_keyboard_focus_changed(&mut self, old_sid: SurfaceId, new_sid: SurfaceId) {
        if old_sid != SurfaceId::invalid() {
            if let Some(surface_info) = self.sid_to_surface_info_dict.get(&old_sid) {
                if let Some(surface_oid) = surface_info.surface_oid {
                    for keyboard_oid in self.keyboard_oids.iter() {
                        let serial = self.socket.get_next_serial();
                        send!(wl_keyboard::leave(&self.socket, *keyboard_oid, serial, surface_oid));

                        if let Some(window_info) = self.coordinator.get_surface(old_sid) {
                            self.on_surface_reconfigured(old_sid,
                                                         window_info.desired_size,
                                                         window_info.state_flags);
                        }
                    }
                }
            }
        }

        if new_sid != SurfaceId::invalid() {
            if let Some(surface_info) = self.sid_to_surface_info_dict.get(&new_sid) {
                if let Some(surface_oid) = surface_info.surface_oid {
                    for keyboard_oid in self.keyboard_oids.iter() {
                        let serial = self.socket.get_next_serial();

                        // TODO: Pass correct keys on keyboard enter.
                        let keys: [u32; 0] = [0; 0];

                        send!(wl_keyboard::enter(&self.socket,
                                                 *keyboard_oid,
                                                 serial,
                                                 surface_oid,
                                                 &keys[..]));

                        if let Some(window_info) = self.coordinator.get_surface(new_sid) {
                            self.on_surface_reconfigured(new_sid,
                                                         window_info.desired_size,
                                                         window_info.state_flags);
                        }
                    }
                }
            }
        }
    }

    /// Nothing to do here. `Engine` handles the request.
    fn on_transfer_offered(&mut self) {}

    fn on_transfer_requested(&mut self, mime_type: String, fd: RawFd) {
        if let Some(data_source_oid) = self.data_source_oid {
            send!(wl_data_source::send(&self.socket, data_source_oid, &mime_type, fd));
        }
    }

    fn on_surface_reconfigured(&self,
                               sid: SurfaceId,
                               size: Size,
                               state_flags: surface_state::SurfaceState) {
        if let Some(info) = self.sid_to_surface_info_dict.get(&sid) {
            if let Some(shell_surface) = info.shell_surface_oid {
                match shell_surface {
                    ShellSurfaceOid::Shell(shell_surface_oid) => {
                        send!(wl_shell_surface::configure(&self.socket,
                                                          shell_surface_oid,
                                                          wl_shell_surface::resize::NONE,
                                                          size.width as i32,
                                                          size.height as i32));
                    }
                    ShellSurfaceOid::ZxdgToplevelV6(shell_surface_oid, shell_toplevel_oid) => {
                        let mut pos = 0;
                        let mut states: [u32; 2] = [0; 2];
                        if state_flags.intersects(surface_state::MAXIMIZED) {
                            states[pos] = zxdg_toplevel_v6::state::MAXIMIZED;
                            pos += 1;
                        }
                        if sid == self.coordinator.get_keyboard_focused_sid() {
                            states[pos] = zxdg_toplevel_v6::state::ACTIVATED;
                            pos += 1;
                        }
                        send!(zxdg_toplevel_v6::configure(&self.socket,
                                                          shell_toplevel_oid,
                                                          size.width as i32,
                                                          size.height as i32,
                                                          &states[0..pos]));
                        let serial = self.socket.get_next_serial();
                        send!(zxdg_surface_v6::configure(&self.socket, shell_surface_oid, serial));
                    }
                }
            } else {
                log_warn3!("Received reconfiguration request for surface {:?} \
                           which is not in shell",
                           sid);
            }
        }
    }

    fn on_screenshot_done(&mut self) {
        if let Some(screenshooter_oid) = self.screenshooter_oid {
            if let Some(ref mut screenshot_memory) = self.screenshot_memory {
                let screenshot = self.coordinator.take_screenshot_buffer();
                if let Some(ref screenshot) = screenshot {
                    if let Err(err) = unsafe { screenshot_memory.absorb(screenshot) } {
                        log_warn1!("Screenshot: {:?}", err);
                    }
                } else {
                    log_warn1!("Screenshot: buffer not found");
                }
            }
            send!(weston_screenshooter::done(&self.get_socket(), screenshooter_oid));
            self.mediator.borrow_mut().register_screenshoter(None);
        }
    }
}

// -------------------------------------------------------------------------------------------------

impl Drop for Proxy {
    fn drop(&mut self) {
        self.terminate();
    }
}

// -------------------------------------------------------------------------------------------------
