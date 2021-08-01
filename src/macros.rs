macro_rules! test_println {
    ($($arg:tt)*) => {
        if cfg!(test) && cfg!(slab_print) {
            if std::thread::panicking() {
                // getting the thread ID while panicking doesn't seem to play super nicely with loom's
                // mock lazy_static...
                println!("[PANIC {:>17}:{:<3}] {}", file!(), line!(), format_args!($($arg)*))
            } else {
                println!("[{:?} {:>17}:{:<3}] {}", crate::Tid::<crate::DefaultConfig>::current(), file!(), line!(), format_args!($($arg)*))
            }
        }
    }
}

#[cfg(all(test, loom))]
macro_rules! test_dbg {
    ($e:expr) => {
        match $e {
            e => {
                test_println!("{} = {:?}", stringify!($e), &e);
                e
            }
        }
    };
}
