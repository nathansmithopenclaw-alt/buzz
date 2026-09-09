//! Use the system Bonjour API so macOS can present its Local Network prompt.
//! Raw multicast discovery alone does not report Bonjour's policy-denied state.
#[cfg(target_os = "macos")]
pub(super) fn denied() -> bool {
    use std::ffi::{c_char, c_void};
    type Service = *mut c_void;
    type Reply = unsafe extern "C" fn(
        Service,
        u32,
        u32,
        i32,
        *const c_char,
        *const c_char,
        *const c_char,
        *mut c_void,
    );
    #[link(name = "dns_sd")]
    unsafe extern "C" {
        fn DNSServiceBrowse(
            service: *mut Service,
            flags: u32,
            interface: u32,
            kind: *const c_char,
            domain: *const c_char,
            reply: Reply,
            context: *mut c_void,
        ) -> i32;
        fn DNSServiceRefSockFD(service: Service) -> i32;
        fn DNSServiceProcessResult(service: Service) -> i32;
        fn DNSServiceRefDeallocate(service: Service);
    }
    unsafe extern "C" fn reply(
        _: Service,
        _: u32,
        _: u32,
        error: i32,
        _: *const c_char,
        _: *const c_char,
        _: *const c_char,
        context: *mut c_void,
    ) {
        // DNSServiceProcessResult invokes this synchronously on this thread.
        // The context remains alive until the service is deallocated below.
        unsafe { *(context.cast::<i32>()) = error };
    }
    let mut service = std::ptr::null_mut();
    let mut error = 0_i32;
    // Strings are static NUL-terminated literals. No service reference escapes;
    // callback context is used only while processing the bounded browse below.
    unsafe {
        let initial = DNSServiceBrowse(
            &mut service,
            0,
            0,
            c"_buzz-model._tcp".as_ptr(),
            c"local.".as_ptr(),
            reply,
            (&mut error as *mut i32).cast(),
        );
        if initial != 0 {
            return initial == -65570;
        }
        let mut fd = libc::pollfd {
            fd: DNSServiceRefSockFD(service),
            events: libc::POLLIN,
            revents: 0,
        };
        if fd.fd >= 0 && libc::poll(&mut fd, 1, 2000) > 0 && fd.revents & libc::POLLIN != 0 {
            let result = DNSServiceProcessResult(service);
            if result != 0 {
                error = result;
            }
        }
        DNSServiceRefDeallocate(service);
    }
    error == -65570
}
#[cfg(not(target_os = "macos"))]
pub(super) fn denied() -> bool {
    false
}
