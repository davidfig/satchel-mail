//! Ask Apple Maps for place suggestions from a terminal, the way the Mac app
//! does: `cargo run -p satchel-providers --example apple_places -- "blue bottle"`.
//! The main thread runs a CFRunLoop for MapKit; queries go to it through GCD.

#[cfg(target_os = "macos")]
fn main() {
    use std::ffi::c_void;

    use satchel_core::provider::PlaceSearch;
    use satchel_providers::places::apple_maps::AppleMaps;

    unsafe extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRunLoopRun();
    }
    extern "C" fn run(ctx: *mut c_void) {
        // SAFETY: made from this exact box below.
        let f = unsafe { Box::from_raw(ctx as *mut Box<dyn FnOnce() + Send>) };
        f();
    }

    let queries: Vec<String> = std::env::args().skip(1).collect();
    std::thread::spawn(move || {
        let maps = AppleMaps::new(Box::new(|f| {
            let ctx = Box::into_raw(Box::new(f)) as *mut c_void;
            // SAFETY: the main queue is always there; `run` takes the box back.
            unsafe { dispatch_async_f(&raw const _dispatch_main_q, ctx, run) };
            Ok(())
        }));
        let rt = tokio::runtime::Runtime::new().unwrap();
        for q in if queries.is_empty() { vec!["coffee".to_string()] } else { queries } {
            println!("{q}:");
            match rt.block_on(async { tokio::time::timeout(std::time::Duration::from_secs(5), maps.suggest(&q)).await }) {
                Ok(Ok(found)) => found.iter().for_each(|s| println!("  {} | {}", s.label, s.detail.as_deref().unwrap_or(""))),
                Ok(Err(e)) => println!("  error: {e:#}"),
                Err(_) => println!("  timed out"),
            }
        }
        std::process::exit(0);
    });
    // SAFETY: plain CoreFoundation call on the main thread.
    unsafe { CFRunLoopRun() };
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Apple Maps is only available on macOS");
}
