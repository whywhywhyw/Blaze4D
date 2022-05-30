use std::panic::{catch_unwind, RefUnwindSafe, UnwindSafe};
use std::process::exit;
use ash::vk;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::platform::run_return::EventLoopExtRunReturn;
use crate::b4d::{B4DVertexFormat, Blaze4D};
use crate::glfw_surface::GLFWSurfaceProvider;
use crate::prelude::{Mat4f32, UUID, Vec2u32};

use crate::renderer::emulator::{MeshData, PassRecorder, StaticMeshId};
use crate::vk::objects::surface::SurfaceProvider;
use crate::window::WinitWindow;

#[repr(C)]
#[derive(Debug)]
struct CMeshData {
    vertex_data_ptr: *const u8,
    vertex_data_len: u64,
    index_data_ptr: *const u8,
    index_data_len: u64,
    vertex_stride: u32,
    index_count: u32,
}

impl CMeshData {
    unsafe fn to_mesh_data(&self) -> MeshData {
        if self.vertex_data_ptr.is_null() {
            log::error!("Vertex data pointer is null");
            panic!();
        }
        if self.index_data_ptr.is_null() {
            log::error!("Index data pointer is null");
            panic!();
        }

        MeshData {
            vertex_data: std::slice::from_raw_parts(self.vertex_data_ptr, self.vertex_data_len as usize),
            index_data: std::slice::from_raw_parts(self.index_data_ptr, self.index_data_len as usize),
            vertex_stride: self.vertex_stride,
            index_count: self.index_count,
            index_type: vk::IndexType::UINT16,
        }
    }
}

#[repr(C)]
#[derive(Debug)]
struct CB4DVertexFormat {
    topology: i32,
    stride: u32,
    position_offset: u32,
    position_format: i32,
}

impl CB4DVertexFormat {
    fn to_b4d_vertex_format(&self) -> B4DVertexFormat {
        B4DVertexFormat {
            topology: vk::PrimitiveTopology::from_raw(self.topology),
            stride: self.stride,
            position: (self.position_offset, vk::Format::from_raw(self.position_format)),
            color: None,
            uv: None
        }
    }
}

/// Creates a new [`Blaze4D`] instance.
///
/// This function will take ownership of the provided surface and vertex format set builder. The
/// pointers must not be used again afterwards.
#[no_mangle]
unsafe extern "C" fn b4d_init(surface: *mut GLFWSurfaceProvider, enable_validation: u32) -> *mut Blaze4D {
    catch_unwind(|| {
        if surface.is_null() {
            log::error!("Passed null surface to b4d_init");
            exit(1);
        }

        let surface_provider: Box<dyn SurfaceProvider> = Box::from_raw(surface);

        let enable_validation = enable_validation != 0;

        Box::leak(Box::new(Blaze4D::new(surface_provider, enable_validation)))
    }).unwrap_or_else(|_| {
        log::error!("panic in b4d_init");
        exit(1);
    })
}

/// Destroys a [`Blaze4D`] instance.
#[no_mangle]
unsafe extern "C" fn b4d_destroy(b4d: *mut Blaze4D) {
    catch_unwind(|| {
        if b4d.is_null() {
            log::error!("Passed null to b4d_destroy");
            exit(1);
        }
        Box::from_raw(b4d);
    }).unwrap_or_else(|_| {
        log::error!("panic in b4d_destroy");
        exit(1);
    })
}

#[no_mangle]
unsafe extern "C" fn b4d_set_vertex_formats(b4d: *const Blaze4D, formats_ptr: *const CB4DVertexFormat, formats_len: u32) {
    catch_unwind(|| {
        let b4d = b4d.as_ref().unwrap_or_else(|| {
            log::error!("Passed null b4d to b4d_create_static_mesh");
            exit(1);
        });
        if formats_ptr.is_null() {
            log::error!("Passed null formats_ptr to b4d_set_vertex_formats");
            exit(1);
        }
        let formats = std::slice::from_raw_parts(formats_ptr, formats_len as usize);

        log::error!("Formats: {:?}", formats);

        let formats = formats.iter().map(|format| format.to_b4d_vertex_format()).collect();

        b4d.set_emulator_vertex_formats(formats);
    }).unwrap_or_else(|_| {
        log::error!("panic in b4d_set_vertex_formats");
        exit(1);
    })
}

#[no_mangle]
unsafe extern "C" fn b4d_create_static_mesh(b4d: *const Blaze4D, data: *const CMeshData) -> u64 {
    catch_unwind(|| {
        let b4d = b4d.as_ref().unwrap_or_else(|| {
            log::error!("Passed null b4d to b4d_create_static_mesh");
            exit(1);
        });
        let data = data.as_ref().unwrap_or_else(|| {
            log::error!("Passed null mesh data to b4d_create_static_mesh");
            exit(1);
        });

        let mesh_data = data.to_mesh_data();

        b4d.create_static_mesh(&mesh_data).as_uuid().get_raw()
    }).unwrap_or_else(|_| {
        log::error!("panic in b4d_create_static_mesh");
        exit(1);
    })
}

#[no_mangle]
unsafe extern "C" fn b4d_destroy_static_mesh(b4d: *const Blaze4D, mesh_id: u64) {
    catch_unwind(|| {
        let b4d = b4d.as_ref().unwrap_or_else(|| {
            log::error!("Passed null b4d to b4d_destroy_static_mesh");
            exit(1);
        });

        b4d.drop_static_mesh(StaticMeshId::from_uuid(UUID::from_raw(mesh_id)));
    }).unwrap_or_else(|_| {
        log::error!("panic in b4d_destroy_static_mesh");
        exit(1);
    })
}

/// Calls [`Blaze4D::try_start_frame`].
///
/// If [`Blaze4D::try_start_frame`] returns [`None`] this function returns null.
#[no_mangle]
unsafe extern "C" fn b4d_start_frame(b4d: *mut Blaze4D, window_width: u32, window_height: u32) -> *mut PassRecorder {
    catch_unwind(|| {
        let b4d = b4d.as_mut().unwrap_or_else(|| {
            log::error!("Passed null b4d to b4d_start_frame");
            exit(1);
        });

        let frame = b4d.try_start_frame(Vec2u32::new(window_width, window_height));
        frame.map_or(std::ptr::null_mut(), |recorder| {
            Box::leak(Box::new(recorder))
        })
    }).unwrap_or_else(|_| {
        log::error!("panic in b4d_start_frame");
        exit(1);
    })
}

#[no_mangle]
unsafe extern "C" fn b4d_pass_set_model_view_matrix(pass: *mut PassRecorder, matrix: *const Mat4f32) {
    catch_unwind(|| {
        let pass = pass.as_mut().unwrap_or_else(|| {
            log::error!("Passed null pass to b4d_pass_set_model_view_matrix");
            exit(1);
        });
        let matrix = matrix.as_ref().unwrap_or_else(|| {
            log::error!("Passed null matrix to b4d_pass_set_model_view_matrix");
            exit(1);
        });

        pass.set_model_view_matrix(matrix);
    }).unwrap_or_else(|_| {
        log::error!("panic in b4d_pass_set_model_view_matrix");
        exit(1);
    })
}

#[no_mangle]
unsafe extern "C" fn b4d_pass_set_projection_matrix(pass: *mut PassRecorder, matrix: *const Mat4f32) {
    catch_unwind(|| {
        let pass = pass.as_mut().unwrap_or_else(|| {
            log::error!("Passed null pass to b4d_pass_set_projection_matrix");
            exit(1);
        });
        let matrix = matrix.as_ref().unwrap_or_else(|| {
            log::error!("Passed null matrix to b4d_pass_set_projection_matrix");
            exit(1);
        });

        pass.set_projection_matrix(matrix);
    }).unwrap_or_else(|_| {
        log::error!("panic in b4d_pass_set_model_view_matrix");
        exit(1);
    })
}

#[no_mangle]
unsafe extern "C" fn b4d_pass_draw_static(pass: *mut PassRecorder, mesh_id: u64, type_id: u32) {
    catch_unwind(|| {
        let pass = pass.as_mut().unwrap_or_else(|| {
            log::error!("Passed null pass to b4d_pass_draw_static");
            exit(1);
        });

        pass.draw_static(StaticMeshId::from_uuid(UUID::from_raw(mesh_id)), type_id);
    }).unwrap_or_else(|_| {
        log::error!("panic in b4d_pass_draw_static");
        exit(1);
    })
}

#[no_mangle]
unsafe extern "C" fn b4d_pass_draw_immediate(pass: *mut PassRecorder, data: *const CMeshData, type_id: u32) {
    catch_unwind(|| {
        let pass = pass.as_mut().unwrap_or_else(|| {
            log::error!("Passed null pass to b4d_pass_draw_immediate");
            exit(1);
        });
        let data = data.as_ref().unwrap_or_else(|| {
            log::error!("Passed null mesh data to b4d_pass_draw_immediate");
            exit(1);
        });

        let mesh_data = data.to_mesh_data();

        pass.draw_immediate(&mesh_data, type_id);
    }).unwrap_or_else(|_| {
        log::error!("panic in b4d_pass_draw_immediate");
        exit(1);
    })
}

#[no_mangle]
unsafe extern "C" fn b4d_end_frame(recorder: *mut PassRecorder) {
    catch_unwind(|| {
        if recorder.is_null() {
            log::error!("Passed null to b4d_end_frame");
            exit(1);
        }
        Box::from_raw(recorder);
    }).unwrap_or_else(|_| {
        log::error!("panic in b4d_end_frame");
        exit(1);
    })
}