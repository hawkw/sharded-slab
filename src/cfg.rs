use crate::page::{slot::Generation, Addr};
use crate::Pack;
use std::{fmt, marker::PhantomData};

pub trait Config: Sized {
    const MAX_THREADS: usize = DefaultConfig::MAX_THREADS;
    const MAX_PAGES: usize = DefaultConfig::MAX_PAGES;
    const INITIAL_PAGE_SIZE: usize = DefaultConfig::INITIAL_PAGE_SIZE;
    const RESERVED_BITS: usize = 0;
}

pub(crate) trait CfgPrivate: Config {
    const USED_BITS: usize = Generation::<Self>::LEN + Generation::<Self>::SHIFT;
    const INITIAL_SZ: usize = next_pow2(Self::INITIAL_PAGE_SIZE);
    const MAX_SHARDS: usize = next_pow2(Self::MAX_THREADS);
    const ADDR_INDEX_SHIFT: usize = Self::INITIAL_SZ.trailing_zeros() as usize + 1;

    fn page_size(n: usize) -> usize {
        Self::INITIAL_SZ * 2usize.pow(n as _)
    }

    fn debug() -> DebugConfig<Self> {
        DebugConfig { _cfg: PhantomData }
    }

    fn validate() {
        assert!(
            Self::INITIAL_SZ.is_power_of_two(),
            "invalid Config: {:#?}",
            Self::debug(),
        );
        assert!(
            Self::INITIAL_SZ <= Addr::<Self>::BITS,
            "invalid Config: {:#?}",
            Self::debug()
        );
        assert!(
            Self::USED_BITS <= WIDTH,
            "invalid Config: {:#?}\ntotal number of bits per index is too large to fit in a word!",
            Self::debug()
        );

        assert!(
            WIDTH - Self::USED_BITS >= Self::RESERVED_BITS,
            "invalid Config: {:#?}\nindices are too large to fit reserved bits!",
            Self::debug()
        );
    }
}

pub(crate) trait Unpack: Config {
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

impl<C: Config> Unpack for C {}
impl<C: Config> CfgPrivate for C {}

#[derive(Copy, Clone)]
pub struct DefaultConfig {
    _p: (),
}

pub(crate) struct DebugConfig<C: Config> {
    _cfg: PhantomData<fn(C)>,
}

pub(crate) const WIDTH: usize = std::mem::size_of::<usize>() * 8;

pub(crate) const fn make_mask(bits: usize) -> usize {
    let shift = 1 << (bits - 1);
    shift | (shift - 1)
}

pub(crate) const fn next_pow2(n: usize) -> usize {
    let pow2 = n.count_ones() == 1;
    let zeros = n.leading_zeros();
    1 << (WIDTH - zeros as usize - pow2 as usize)
}

// === impl DefaultConfig ===
impl Config for DefaultConfig {
    const INITIAL_PAGE_SIZE: usize = 32;

    #[cfg(target_pointer_width = "64")]
    const MAX_THREADS: usize = 4096;
    #[cfg(target_pointer_width = "32")]
    const MAX_THREADS: usize = 2048;

    const MAX_PAGES: usize = WIDTH / 2;
}

impl<C: Config> fmt::Debug for DebugConfig<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("initial_page_size", &C::INITIAL_SZ)
            .field("max_shards", &C::MAX_SHARDS)
            .field("max_pages", &C::MAX_PAGES)
            .field("used_bits", &C::USED_BITS)
            .field("reserved_bits", &C::RESERVED_BITS)
            .field("pointer_width", &WIDTH)
            .finish()
    }
}
