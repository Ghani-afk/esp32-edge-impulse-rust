#![no_std]

pub enum Void {}

impl core::fmt::Debug for Void {
    fn fmt(&self, _: &mut core::fmt::Formatter) -> core::fmt::Result {
        match *self {}
    }
}
impl core::fmt::Display for Void {
    fn fmt(&self, _: &mut core::fmt::Formatter) -> core::fmt::Result {
        match *self {}
    }
}

pub trait ResultVoidExt<T> {
    fn void_unwrap(self) -> T;
}
impl<T> ResultVoidExt<T> for Result<T, Void> {
    fn void_unwrap(self) -> T {
        match self {
            Ok(v) => v,
            Err(e) => match e {},
        }
    }
}
