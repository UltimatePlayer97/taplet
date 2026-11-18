use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::mem::size_of;

use rustix::io::Errno;
use rustix::ioctl::{self, Ioctl, IoctlOutput, Opcode};

pub const O_NONBLOCK: i32 = rustix::fs::OFlags::NONBLOCK.bits() as i32;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct InputEvent {
    pub time: [std::os::raw::c_long; 2],
    pub type_: u16,
    pub code: u16,
    pub value: i32,
}

struct ValueIoctl {
    request: Opcode,
    value: i32,
}

unsafe impl Ioctl for ValueIoctl {
    type Output = ();

    const IS_MUTATING: bool = false;

    fn opcode(&self) -> Opcode {
        self.request
    }

    fn as_ptr(&mut self) -> *mut c_void {
        self.value as usize as *mut c_void
    }

    unsafe fn output_from_ptr(
        _: IoctlOutput,
        _: *mut c_void,
    ) -> Result<Self::Output, Errno> {
        Ok(())
    }
}

struct BufferIoctl<'a> {
    request: Opcode,
    buffer: &'a mut [u8],
}

unsafe impl Ioctl for BufferIoctl<'_> {
    type Output = ();

    const IS_MUTATING: bool = true;

    fn opcode(&self) -> Opcode {
        self.request
    }

    fn as_ptr(&mut self) -> *mut c_void {
        self.buffer.as_mut_ptr().cast()
    }

    unsafe fn output_from_ptr(
        _: IoctlOutput,
        _: *mut c_void,
    ) -> Result<Self::Output, Errno> {
        Ok(())
    }
}

pub fn ioctl_value(file: &File, request: Opcode, value: i32) -> io::Result<()> {
    unsafe { ioctl::ioctl(file, ValueIoctl { request, value }) }.map_err(Into::into)
}

pub fn ioctl_read(file: &File, request: Opcode, buffer: &mut [u8]) -> io::Result<()> {
    unsafe {
        ioctl::ioctl(
            file,
            BufferIoctl {
                request,
                buffer,
            },
        )
    }
    .map_err(Into::into)
}

pub fn write_struct<T>(file: &File, value: &T) -> io::Result<()> {
    let bytes = unsafe {
        std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>())
    };
    write_all(file, bytes)
}

pub fn write_all(file: &File, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        match rustix::io::write(file, bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => bytes = &bytes[written..],
            Err(rustix::io::Errno::INTR) => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}
