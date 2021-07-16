fn main() {
    autocfg::new().emit_expression_cfg(
        r#"{ struct A<const N: usize>([(); N]); A([]) }"#,
        "has_const_generics",
    );
}
