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
    let slab = Slab::builder().initial_page_size(4).finish();

    for i in 0..4096 {
        let k = slab.insert(i).expect("insert");
        assert_eq!(slab.get(k).expect("get"), &i);
    }
}
