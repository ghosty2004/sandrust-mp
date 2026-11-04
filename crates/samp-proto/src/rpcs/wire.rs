//! Fixed-width field codec shared by the `payload!` macros of the RPC modules.

use crate::bitstream::{BitReader, BitWriter};
use crate::types::{Vec2, Vec3};

/// A fixed-layout wire field. Integers and floats are little-endian; `bool`
/// is a single bit; `[u32; N]` is `N` consecutive u32.
pub(crate) trait Wire: Sized {
    fn get(r: &mut BitReader) -> Option<Self>;
    fn put(&self, w: &mut BitWriter);
}

macro_rules! wire_via {
    ($($t:ty => $read:ident, $write:ident;)+) => {
        $(impl Wire for $t {
            fn get(r: &mut BitReader) -> Option<Self> {
                r.$read()
            }
            fn put(&self, w: &mut BitWriter) {
                w.$write(*self);
            }
        })+
    };
}

wire_via! {
    bool => read_bit, write_bit;
    u8 => read_u8, write_u8;
    u16 => read_u16, write_u16;
    u32 => read_u32, write_u32;
    i32 => read_i32, write_i32;
    f32 => read_f32, write_f32;
}

impl Wire for Vec2 {
    fn get(r: &mut BitReader) -> Option<Self> {
        Vec2::read(r)
    }
    fn put(&self, w: &mut BitWriter) {
        self.write(w);
    }
}
impl Wire for Vec3 {
    fn get(r: &mut BitReader) -> Option<Self> {
        Vec3::read(r)
    }
    fn put(&self, w: &mut BitWriter) {
        self.write(w);
    }
}
impl<const N: usize> Wire for [u32; N] {
    fn get(r: &mut BitReader) -> Option<Self> {
        let mut a = [0u32; N];
        for v in &mut a {
            *v = r.read_u32()?;
        }
        Some(a)
    }
    fn put(&self, w: &mut BitWriter) {
        for v in self {
            w.write_u32(*v);
        }
    }
}
