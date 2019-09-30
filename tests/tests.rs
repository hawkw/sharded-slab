use sharded_slab::Slab;
#[test]
fn big() {
    let slab = Slab::new();

    for i in 0..10000 {
        let k = slab.insert(i).expect("insert");
        assert_eq!(slab.get(k).expect("get"), &i);
    }
}

#[test]
fn custom_page_sz() {
    struct TinyConfig;

    impl sharded_slab::Params for TinyConfig {
        const MAX_PAGES: usize = 1;
        const INITIAL_PAGE_SIZE: usize = 4;
        const MAX_THREADS: usize = 4096;
    }
    let slab = Slab::<_, TinyConfig>::new_with_config();

    for i in 0..4096 {
        let k = slab.insert(i).expect("insert");
        assert_eq!(slab.get(k).expect("get"), &i);
    }
}
