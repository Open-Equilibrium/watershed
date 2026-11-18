use crate::api::session::run;

#[test]
fn a_synchronous_call_returns_its_result_inside_either_tokio_runtime_flavor() {
    let current = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime starts");
    assert_eq!(current.block_on(async { run(async { 7 }) }), 7);

    let multi = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime starts");
    assert_eq!(multi.block_on(async { run(async { 8 }) }), 8);
}
