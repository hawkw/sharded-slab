use crate::page::{slot::Generation, Addr};
use crate::Pack;
use std::{fmt, marker::PhantomData};

pub trait Params: Sized {
    const MAX_THREADS: usize;
    const MAX_PAGES: usize;
    const INITIAL_PAGE_SIZE: usize;
    const RESERVED_BITS: usize = 0;

    const USED_BITS: usize = Generation::<Self>::LEN + Generation::<Self>::SHIFT;

    const ACTUAL_INITIAL_SZ: usize = next_pow2(Self::INITIAL_PAGE_SIZE);

    const MAX_SHARDS: usize = next_pow2(Self::MAX_THREADS);

    const ADDR_INDEX_SHIFT: usize = Self::ACTUAL_INITIAL_SZ.trailing_zeros() as usize + 1;

    fn page_size(n: usize) -> usize {
        Self::ACTUAL_INITIAL_SZ * 2usize.pow(n as u32)
    }

    fn validate() {
        assert!(Self::ACTUAL_INITIAL_SZ.is_power_of_two());
        assert!(Self::ACTUAL_INITIAL_SZ <= Addr::<Self>::BITS);
        assert!(
            Self::USED_BITS <= WIDTH,
            "total number of bits per index is too large to fit in a word!"
        );

        assert!(
            WIDTH - Self::USED_BITS >= Self::RESERVED_BITS,
            "indices are too large to fit reserved bits!"
        );
    }
}

pub(crate) trait Unpack: Params {
    #[inline(always)]
    fn unpack<A: Pack<Self>>(packed: usize) -> A {
        A::from_packed(packed)
    }

    #[inline(always)]
    fn unpack_addr(packed: usize) -> Addr<Self> {
        Self::unpack(packed)
    }

    #[inline(always)]
    fn unpack_tid(packed: usize) -> crate::Tid<Self> {
        Self::unpack(packed)
    }

    #[inline(always)]
    fn unpack_gen(packed: usize) -> Generation<Self> {
        Self::unpack(packed)
    }
}

impl<P: Params> Unpack for P {}

#[derive(Copy, Clone)]
pub struct DefaultParams {
    _p: (),
}

pub struct Config<P: Params> {
    _params: PhantomData<fn(P)>,
}

#[cfg(target_pointer_width = "32")]
pub(crate) const WIDTH: usize = 32;
#[cfg(target_pointer_width = "64")]
pub(crate) const WIDTH: usize = 64;

#[cfg(target_pointer_width = "64")]
pub(crate) const fn make_mask(bits: u32) -> usize {
    std::usize::MAX >> (WIDTH - bits as usize)
}

pub(crate) const fn next_pow2(n: usize) -> usize {
    let pow2 = n.count_ones() == 1;
    let ctlz = n.leading_zeros();
    let bits = std::mem::size_of::</* T */ usize>() * 8;
    1 << (bits - ctlz as usize - pow2 as usize)
}

// === impl DefaultParams ===
impl Params for DefaultParams {
    const INITIAL_PAGE_SIZE: usize = 32;

    #[cfg(target_pointer_width = "64")]
    const MAX_THREADS: usize = 4096;
    #[cfg(target_pointer_width = "32")]
    const MAX_THREADS: usize = 2048;

    const MAX_PAGES: usize = WIDTH / 2;
}

// impl<P: Params> fmt::Debug for Config<P> {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         f.debug_struct("Config")
//             .field("initial_page_size", &Self::initial_page_size())
//             .field("max_shards", &Self::max_shards())
//             .field("max_pages", &Self::MAX_PAGES)
//             .field("used_bits", &Self::USED_BITS)
//             .field("reserved_bits", &Self::RESERVED_BITS)
//             .field("pointer_width", &WIDTH)
//             .finish()
//     }
// }

// impl<P: Params> PartialEq for Config<P> {
//     #[inline(always)]
//     fn eq(&self, _: &Self) -> bool {
//         true
//     }
// }

// impl<P: Params> Eq for Config<P> {}

// impl<P: Params> PartialOrd for Config<P> {
//     #[inline(always)]
//     fn partial_cmp(&self, _: &Self) -> Option<std::cmp::Ordering> {
//         Some(std::cmp::Ordering::Equal)
//     }
// }

// impl<P: Params> Ord for Config<P> {
//     #[inline(always)]
//     fn cmp(&self, _: &Self) -> std::cmp::Ordering {
//         std::cmp::Ordering::Equal
//     }
// }
// impl<P: Params> Clone for Config<P> {
//     fn clone(&self) -> Self {
//         Self::new()
//     }
// }
// impl<P: Params> Copy for Config<P> {}
