fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    rt.block_on(async {
        tokio::task::spawn_blocking(|| {
            let res = std::panic::catch_unwind(|| {
                tokio::task::block_in_place(|| {
                    println!("In block_in_place");
                });
            });
            if let Err(e) = res {
                if let Some(s) = e.downcast_ref::<&str>() {
                    println!("Panic: {}", s);
                } else if let Some(s) = e.downcast_ref::<String>() {
                    println!("Panic: {}", s);
                }
            }
        }).await.unwrap();
    });
}
