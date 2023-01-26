use raw_window_handle::OrbitalWindowHandle;
use std::{cmp, slice, str};

use crate::SoftBufferError;

struct OrbitalMap {
    address: usize,
    size: usize,
}

impl OrbitalMap {
    unsafe fn new(fd: usize, size_unaligned: usize) -> syscall::Result<Self> {
        // Page align size
        let pages = (size_unaligned + syscall::PAGE_SIZE - 1) / syscall::PAGE_SIZE;
        let size = pages * syscall::PAGE_SIZE;

        // Map window buffer
        let address = unsafe {
            syscall::fmap(
                fd,
                &syscall::Map {
                    offset: 0,
                    size,
                    flags: syscall::PROT_READ | syscall::PROT_WRITE,
                    address: 0,
                },
            )?
        };

        Ok(Self { address, size })
    }
}

impl Drop for OrbitalMap {
    fn drop(&mut self) {
        unsafe {
            // Unmap window buffer on drop
            syscall::funmap(self.address, self.size).expect("failed to unmap orbital window");
        }
    }
}

pub struct OrbitalImpl {
    handle: OrbitalWindowHandle,
    width: u32,
    height: u32,
    buffer: Vec<u32>,
}

impl OrbitalImpl {
    pub fn new(handle: OrbitalWindowHandle) -> Result<Self, SoftBufferError> {
        Ok(Self {
            handle,
            width: 0,
            height: 0,
            buffer: Vec::new(),
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), SoftBufferError> {
        self.width = width;
        self.height = height;
        Ok(())
    }

    pub fn buffer_mut(&mut self) -> Result<BufferImpl, SoftBufferError> {
        self.buffer
            .resize(self.width as usize * self.height as usize, 0);
        Ok(BufferImpl { imp: self })
    }

    fn set_buffer(&self, buffer: &[u32], width_u32: u32, height_u32: u32) {
        let window_fd = self.handle.window as usize;

        // Read the current width and size
        let mut window_width = 0;
        let mut window_height = 0;
        {
            let mut buf: [u8; 4096] = [0; 4096];
            let count = syscall::fpath(window_fd, &mut buf).unwrap();
            let path = str::from_utf8(&buf[..count]).unwrap();
            // orbital:/x/y/w/h/t
            let mut parts = path.split('/').skip(3);
            if let Some(w) = parts.next() {
                window_width = w.parse::<usize>().unwrap_or(0);
            }
            if let Some(h) = parts.next() {
                window_height = h.parse::<usize>().unwrap_or(0);
            }
        }

        {
            // Map window buffer
            let window_map =
                unsafe { OrbitalMap::new(window_fd, window_width * window_height * 4) }
                    .expect("failed to map orbital window");

            // Window buffer is u32 color data in 0xAABBGGRR format
            let window_data = unsafe {
                slice::from_raw_parts_mut(
                    window_map.address as *mut u32,
                    window_width * window_height,
                )
            };

            // Copy each line, cropping to fit
            let width = width_u32 as usize;
            let height = height_u32 as usize;
            let min_width = cmp::min(width, window_width);
            let min_height = cmp::min(height, window_height);
            for y in 0..min_height {
                let offset_buffer = y * width;
                let offset_data = y * window_width;
                window_data[offset_data..offset_data + min_width]
                    .copy_from_slice(&buffer[offset_buffer..offset_buffer + min_width]);
            }

            // Window buffer map is dropped here
        }

        // Tell orbital to show the latest window data
        syscall::fsync(window_fd).expect("failed to sync orbital window");
    }
}

pub struct BufferImpl<'a> {
    imp: &'a mut OrbitalImpl,
}

impl<'a> BufferImpl<'a> {
    pub fn pixels(&self) -> &[u32] {
        &self.imp.buffer
    }

    pub fn pixels_mut(&mut self) -> &mut [u32] {
        &mut self.imp.buffer
    }

    pub fn present(self) -> Result<(), SoftBufferError> {
        self.imp
            .set_buffer(&self.imp.buffer, self.imp.width, self.imp.height);
        Ok(())
    }
}
