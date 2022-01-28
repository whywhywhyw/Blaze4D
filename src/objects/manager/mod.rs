//! Management of vulkan objects.
//!
//! Contains structs and enums to manage creation, access to and destruction of vulkan objects.
//!
//! Access to objects is controlled using synchronization groups. All objects belonging to a
//! synchronization group are accessed as one unit protected by a single timeline semaphore.
//!
//! Allocation and destruction of objects is managed through object sets. A objects set is a
//! collection of objects that have the same lifetime. All objects are created when creating the set
//! and all objects are destroyed only when the entire set is destroyed. All objects of a set
//! belong to the same synchronization group.
//!
//! Both synchronization groups as well as objects sets are managed by smart pointers eliminating
//! the need for manual lifetime management. Object sets keep a reference to their synchronization
//! group internally meaning that if a synchronization group is needed only for a single objects set
//! it suffices to keep the object set alive to also ensure the synchronization group stays alive.
//!
//! Multiple object sets can be accessed in a sequentially consistent manner by using
//! synchronization group sets. This is required to prevent deadlock situations when trying to
//! access multiple sets for the same operation.

pub(super) mod synchronization_group;
pub(super) mod object_set;

mod allocator;
mod resource_object_set;
mod swapchain_object_set;

use std::sync::Arc;

use ash::vk;

use synchronization_group::*;
use crate::objects::manager::allocator::*;
use crate::util::slice_splitter::Splitter;

pub use object_set::ObjectSetProvider;
use crate::objects::manager::resource_object_set::{ObjectCreateError, ResourceObjectCreateMetadata, ResourceObjectCreator, ResourceObjectData, ResourceObjectSetBuilder};
use crate::UUID;

// Internal implementation of the object manager
struct ObjectManagerImpl {
    uuid: UUID,
    device: crate::rosella::DeviceContext,
    allocator: Allocator,
}

impl ObjectManagerImpl {
    fn new(device: crate::rosella::DeviceContext) -> Self {
        let allocator = Allocator::new(device.clone());

        Self{
            uuid: UUID::new(),
            device,
            allocator,
        }
    }

    /// Creates a timeline semaphore for use in a synchronization group
    fn create_group_semaphore(&self, initial_value: u64) -> vk::Semaphore {
        let mut timeline_info = vk::SemaphoreTypeCreateInfo::builder()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(initial_value);
        let info = vk::SemaphoreCreateInfo::builder().push_next(&mut timeline_info);

        unsafe {
            self.device.vk().create_semaphore(&info.build(), None).unwrap()
        }
    }

    /// Destroys a semaphore previously created using [`ObjectManagerImpl::create_timeline_semaphore`]
    fn destroy_group_semaphore(&self, semaphore: vk::Semaphore) {
        unsafe {
            self.device.vk().destroy_semaphore(semaphore, None)
        }
    }

    fn create_resource_objects(&self, objects: &mut Box<[ResourceObjectCreateMetadata]>) -> Result<(), ObjectCreateError> {
        for i in 0..objects.len() {
            let (splitter, current) = Splitter::new(objects.as_mut(), i);
            current.create(&self.device, &self.allocator, &splitter)?
        }
        Ok(())
    }

    fn abort_resource_objects(&self, objects: &mut Box<[ResourceObjectCreateMetadata]>) {
        for object in objects.iter_mut().rev() {
            object.abort(&self.device, &self.allocator)
        }
    }

    fn reduce_resource_objects(&self, objects: Box<[ResourceObjectCreateMetadata]>) -> (Box<[ResourceObjectData]>, Box<[Allocation]>){
        let mut data = Vec::with_capacity(objects.len());
        let mut allocations = Vec::new();

        for object in objects.into_vec() {
            let (d, alloc) = object.reduce();
            data.push(d);

            match alloc {
                Some(alloc) => allocations.push(alloc),
                None => {}
            }
        }

        (data.into_boxed_slice(), allocations.into_boxed_slice())
    }

    fn destroy_resource_objects(&self, objects: Box<[ResourceObjectData]>, allocations: Box<[Allocation]>) {
        for object in objects.into_vec().into_iter().rev() {
            object.destroy(&self.device)
        }
        for allocation in allocations.into_vec() {
            self.allocator.free(allocation)
        }
    }
}

/// Public object manager api.
///
/// This is a smart pointer reference to an internal struct.
pub struct ObjectManager(Arc<ObjectManagerImpl>);

impl ObjectManager {
    /// Creates a new ObjectManager
    pub fn new(device: crate::rosella::DeviceContext) -> Self {
        Self(Arc::new(ObjectManagerImpl::new(device)))
    }

    /// Creates a new synchronization group managed by this object manager
    pub fn create_synchronization_group(&self) -> SynchronizationGroup {
        SynchronizationGroup::new(self.clone(), self.0.create_group_semaphore(0u64))
    }

    /// Creates a new resource object set builder
    ///
    /// #Panics
    /// If the synchronization group belongs to a different object manager.
    pub fn create_resource_object_set(&self, synchronization_group: SynchronizationGroup) -> ResourceObjectSetBuilder {
        if synchronization_group.get_manager() != self {
            panic!("Synchronization group belongs to different object manager");
        }

        ResourceObjectSetBuilder::new(synchronization_group)
    }

    // Internal function that destroys a semaphore created for a synchronization group
    fn destroy_group_semaphore(&self, semaphore: vk::Semaphore) {
        self.0.destroy_group_semaphore(semaphore)
    }

    fn build_resource_objects(&self, mut objects: Box<[ResourceObjectCreateMetadata]>) -> (Box<[ResourceObjectData]>, Box<[Allocation]>) {
        let result = self.0.create_resource_objects(&mut objects);
        if result.is_err() {
            self.0.abort_resource_objects(&mut objects);
            panic!("Error during object creation")
        }

        self.0.reduce_resource_objects(objects)
    }

    fn destroy_resource_objects(&self, objects: Box<[ResourceObjectData]>, allocations: Box<[Allocation]>) {
        self.0.destroy_resource_objects(objects, allocations)
    }
}

impl Clone for ObjectManager {
    fn clone(&self) -> Self {
        Self( self.0.clone() )
    }
}

impl PartialEq for ObjectManager {
    fn eq(&self, other: &Self) -> bool {
        self.0.uuid.eq(&other.0.uuid)
    }
}

impl Eq for ObjectManager {
}

#[cfg(test)]
mod tests {
    use crate::objects;
    use crate::objects::buffer::{BufferDescription, BufferViewDescription};
    use crate::objects::{BufferRange, ImageSize, ImageSpec, ImageSubresourceRange};
    use crate::objects::image::{ImageDescription, ImageViewDescription};
    use super::*;

    fn create() -> ObjectManager {
        let (_, device) = crate::test::make_headless_instance_device();
        ObjectManager::new(device)
    }

    #[test]
    fn create_destroy() {
        let (_, device) = crate::test::make_headless_instance_device();
        let manager = ObjectManager::new(device);
        drop(manager);
    }

    #[test]
    fn create_synchronization_group() {
        let manager = create();
        let group = manager.create_synchronization_group();
        let group2 = manager.create_synchronization_group();

        assert_eq!(group, group);
        assert_eq!(group2, group2);
        assert_ne!(group, group2);

        drop(group2);
        drop(group);
    }

    #[test]
    fn create_buffer() {
        let manager = create();
        let group = manager.create_synchronization_group();
        let mut builder = manager.create_resource_object_set(group);

        let id = builder.add_default_gpu_only_buffer(BufferDescription::new_simple(1024, vk::BufferUsageFlags::TRANSFER_SRC));

        let set = builder.build();

        assert_ne!(set.get_buffer_handle(id), vk::Buffer::null());

        drop(set);
    }

    #[test]
    fn create_buffer_view() {
        let manager = create();
        let group = manager.create_synchronization_group();
        let mut builder = manager.create_resource_object_set(group);

        let buffer = builder.add_default_gpu_only_buffer(BufferDescription::new_simple(1024, vk::BufferUsageFlags::UNIFORM_TEXEL_BUFFER));
        let view = builder.add_internal_buffer_view(BufferViewDescription::new_simple(BufferRange { offset: 0, length: 1024 }, &objects::Format::R16_UNORM), buffer);

        let set = builder.build();

        assert_ne!(set.get_buffer_view_handle(view), vk::BufferView::null());

        drop(set);
    }

    #[test]
    fn create_image() {
        let manager = create();
        let group = manager.create_synchronization_group();
        let mut builder = manager.create_resource_object_set(group);

        let image = builder.add_default_gpu_only_image(ImageDescription::new_simple(
            ImageSpec::new_single_sample(ImageSize::make_2d(32, 32), &objects::Format::R8_UNORM),
            vk::ImageUsageFlags::TRANSFER_SRC)
        );

        let set = builder.build();

        assert_ne!(set.get_image_handle(image), vk::Image::null());

        drop(set);
    }

    #[test]
    fn create_image_view() {
        let manager = create();
        let group = manager.create_synchronization_group();
        let mut builder = manager.create_resource_object_set(group);

        let image = builder.add_default_gpu_only_image(ImageDescription::new_simple(
            ImageSpec::new_single_sample(ImageSize::make_2d(32, 32), &objects::Format::R8_UNORM),
            vk::ImageUsageFlags::SAMPLED)
        );
        let view = builder.add_internal_image_view(ImageViewDescription {
            view_type: vk::ImageViewType::TYPE_2D,
            format: &objects::Format::R8_UNORM,
            components: vk::ComponentMapping::default(),
            subresource_range: ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                mip_level_count: 1,
                base_array_layer: 0,
                array_layer_count: 1,
            }
        }, image);

        let set = builder.build();

        assert_ne!(set.get_image_view_handle(view), vk::ImageView::null());

        drop(set);
    }
}