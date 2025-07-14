#![no_std]
#![no_main]
#![feature(lang_items)]
#![feature(alloc_error_handler)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::mem::{size_of, MaybeUninit};
use core::ptr::null_mut;

// --- ARCHITECTURE-SPECIFIC SYSCALL NUMBERS FOR aarch64 ---
const SYS_WRITE: i64 = 64; const SYS_EXIT: i64 = 93; const SYS_CLOSE: i64 = 57;
const SYS_SOCKET: i64 = 198; const SYS_BIND: i64 = 200; const SYS_LISTEN: i64 = 201;
const SYS_SETSOCKOPT: i64 = 208; const SYS_IO_URING_SETUP: i64 = 425; const SYS_IO_URING_ENTER: i64 = 426;

// --- IO_URING FEATURE FLAGS ---
const IORING_SETUP_SUBMIT_ALL: u32 = 1 << 3;

// --- LINUX ERROR CODES (as negative values) ---
const EINTR: i64 = -4; const EAGAIN: i64 = -11;

// Custom allocator and handlers
struct FastAllocator;
unsafe impl GlobalAlloc for FastAllocator { unsafe fn alloc(&self, layout: Layout) -> *mut u8 { libc::malloc(layout.size()) as *mut u8 } unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) { libc::free(ptr as *mut c_void); } }
#[global_allocator] static ALLOCATOR: FastAllocator = FastAllocator;
#[panic_handler] fn panic(_info: &core::panic::PanicInfo) -> ! { loop {} }
#[alloc_error_handler] fn alloc_error_handler(_layout: Layout) -> ! { loop {} }
#[lang = "eh_personality"] extern "C" fn eh_personality() {}

// io_uring constants and structs
const IORING_ENTER_GETEVENTS: u32 = 1 << 0; const IORING_OP_ACCEPT: u8 = 13; const IORING_OP_RECV: u8 = 15; const IORING_OP_SEND: u8 = 16;
#[repr(C)] struct io_uring_params { sq_entries: u32, cq_entries: u32, flags: u32, sq_thread_cpu: u32, sq_thread_idle: u32, features: u32, wq_fd: u32, resv: [u32; 3], sq_off: io_sqring_offsets, cq_off: io_cqring_offsets, }
#[repr(C)] #[derive(Copy, Clone)] struct io_sqring_offsets { head: u32, tail: u32, ring_mask: u32, ring_entries: u32, flags: u32, dropped: u32, array: u32, resv1: u32, resv2: u64, }
#[repr(C)] #[derive(Copy, Clone)] struct io_cqring_offsets { head: u32, tail: u32, ring_mask: u32, ring_entries: u32, overflow: u32, cqes: u32, flags: u32, resv1: u32, resv2: u64, }
#[repr(C)] #[derive(Copy, Clone)] struct io_uring_sqe { opcode: u8, flags: u8, ioprio: u16, fd: i32, off: u64, addr: u64, len: u32, rw_flags: u32, user_data: u64, pad: [u16; 3], }
#[repr(C)] struct io_uring_cqe { user_data: u64, res: i32, flags: u32, }
#[repr(C)] struct sockaddr_in { sin_family: u16, sin_port: u16, sin_addr: u32, sin_zero: [u8; 8], }

// Pre-computed HTTP responses
static HTTP_200_HELLO: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 13\r\nConnection: keep-alive\r\nServer: ultimate-rust\r\n\r\nHello, World!";
static HTTP_404: &[u8] = b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nConnection: keep-alive\r\nServer: ultimate-rust\r\n\r\nNot Found";

const MAX_CONNECTIONS: usize = 2048; const IO_URING_QUEUE_SIZE: u32 = 1024; const BUFFER_SIZE: usize = 2048;
struct Connection { fd: i32, buffer: [u8; BUFFER_SIZE], active: bool, }
impl Connection { fn new() -> Self { Self { fd: -1, buffer: unsafe { MaybeUninit::<[u8; BUFFER_SIZE]>::uninit().assume_init() }, active: false, } } }

struct UltimateServer {
    ring_fd: i32, listen_fd: i32, connections: Vec<Connection>, free_list: Vec<usize>,
    sq_ring: *mut u8, cq_ring: *mut u8, sqes: *mut io_uring_sqe, sq_array: *mut u32,
    sq_head: *mut u32, sq_tail: *mut u32, sq_mask: u32,
    cq_head: *mut u32, cq_tail: *mut u32, cq_mask: u32, cqes: *mut io_uring_cqe,
}

impl UltimateServer {
    unsafe fn new() -> Result<Self, (i64, &'static str)> {
        let mut params: io_uring_params = core::mem::zeroed();
        params.flags = IORING_SETUP_SUBMIT_ALL;
        
        let ring_fd = libc::syscall(SYS_IO_URING_SETUP, IO_URING_QUEUE_SIZE, &mut params as *mut _ as *mut c_void) as i64;
        if ring_fd < 0 { return Err((ring_fd, "io_uring_setup")); }
        
        let sq_ring_size = params.sq_off.array as usize + params.sq_entries as usize * size_of::<u32>();
        let cq_ring_size = params.cq_off.cqes as usize + params.cq_entries as usize * size_of::<io_uring_cqe>();
        
        // FINAL FIX: Removed MAP_POPULATE flag, which can cause EPERM on some systems.
        let mmap_flags = libc::MAP_SHARED;
        let sq_ring = libc::mmap(null_mut(), sq_ring_size, libc::PROT_READ | libc::PROT_WRITE, mmap_flags, ring_fd as i32, 0) as *mut u8;
        if sq_ring == libc::MAP_FAILED as *mut u8 { return Err((-1, "mmap sq_ring")); }
        let cq_ring = libc::mmap(null_mut(), cq_ring_size, libc::PROT_READ | libc::PROT_WRITE, mmap_flags, ring_fd as i32, 0x8000000) as *mut u8;
        if cq_ring == libc::MAP_FAILED as *mut u8 { return Err((-1, "mmap cq_ring")); }
        let sqes = libc::mmap(null_mut(), params.sq_entries as usize * size_of::<io_uring_sqe>(), libc::PROT_READ | libc::PROT_WRITE, mmap_flags, ring_fd as i32, 0x10000000) as *mut io_uring_sqe;
        if sqes == libc::MAP_FAILED as *mut io_uring_sqe { return Err((-1, "mmap sqes")); }
        
        let listen_fd = libc::syscall(SYS_SOCKET, libc::AF_INET, libc::SOCK_STREAM, 0);
        if listen_fd < 0 { return Err((listen_fd, "socket")); }
        let optval = 1i32;
        let res_reuseaddr = libc::syscall(SYS_SETSOCKOPT, listen_fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, &optval as *const i32 as *const c_void, size_of::<i32>() as u32);
        if res_reuseaddr < 0 { return Err((res_reuseaddr, "setsockopt reuseaddr")); }
        let res_reuseport = libc::syscall(SYS_SETSOCKOPT, listen_fd, libc::SOL_SOCKET, libc::SO_REUSEPORT, &optval as *const i32 as *const c_void, size_of::<i32>() as u32);
        if res_reuseport < 0 { return Err((res_reuseport, "setsockopt reuseport")); }
        
        let addr = sockaddr_in { sin_family: libc::AF_INET as u16, sin_port: 8080u16.to_be(), sin_addr: 0, sin_zero: [0; 8] };
        let res_bind = libc::syscall(SYS_BIND, listen_fd, &addr as *const sockaddr_in as *const c_void, size_of::<sockaddr_in>() as u32);
        if res_bind < 0 { return Err((res_bind, "bind")); }
        let res_listen = libc::syscall(SYS_LISTEN, listen_fd, 1024);
        if res_listen < 0 { return Err((res_listen, "listen")); }
        
        let mut connections = Vec::with_capacity(MAX_CONNECTIONS);
        for _ in 0..MAX_CONNECTIONS { connections.push(Connection::new()); }
        let free_list = (0..MAX_CONNECTIONS).collect();

        Ok(Self {
            ring_fd: ring_fd as i32, listen_fd: listen_fd as i32, connections, free_list, sq_ring, cq_ring, sqes,
            sq_array: sq_ring.add(params.sq_off.array as usize) as *mut u32,
            sq_head: sq_ring.add(params.sq_off.head as usize) as *mut u32,
            sq_tail: sq_ring.add(params.sq_off.tail as usize) as *mut u32,
            sq_mask: *sq_ring.add(params.sq_off.ring_mask as usize).cast(),
            cq_head: cq_ring.add(params.cq_off.head as usize) as *mut u32,
            cq_tail: cq_ring.add(params.cq_off.tail as usize) as *mut u32,
            cq_mask: *cq_ring.add(params.cq_off.ring_mask as usize).cast(),
            cqes: cq_ring.add(params.cq_off.cqes as usize) as *mut io_uring_cqe,
        })
    }
    
    #[inline(always)] unsafe fn submit_sqe(&mut self, sqe: io_uring_sqe) -> bool { let head = core::ptr::read_volatile(self.sq_head); let tail = *self.sq_tail; if tail.wrapping_sub(head) > self.sq_mask { return false; } let index = tail & self.sq_mask; self.sqes.add(index as usize).write(sqe); self.sq_array.add(index as usize).write(index); core::sync::atomic::fence(core::sync::atomic::Ordering::Release); *self.sq_tail = tail.wrapping_add(1); true }
    #[inline(always)] unsafe fn submit_accept(&mut self) -> bool { let sqe = io_uring_sqe { opcode: IORING_OP_ACCEPT, flags: 0, ioprio: 0, fd: self.listen_fd, off: 0, addr: 0, len: 0, rw_flags: 0, user_data: (1u64 << 32), pad: [0; 3], }; self.submit_sqe(sqe) }
    #[inline(always)] unsafe fn submit_recv(&mut self, conn_id: usize) -> bool { let conn = &mut self.connections[conn_id]; let sqe = io_uring_sqe { opcode: IORING_OP_RECV, flags: 0, ioprio: 0, fd: conn.fd, off: 0, addr: conn.buffer.as_mut_ptr() as u64, len: BUFFER_SIZE as u32, rw_flags: 0, user_data: (2u64 << 32) | conn_id as u64, pad: [0; 3], }; self.submit_sqe(sqe) }
    #[inline(always)] unsafe fn submit_send(&mut self, conn_id: usize, response: &'static [u8]) -> bool { let conn = &mut self.connections[conn_id]; let sqe = io_uring_sqe { opcode: IORING_OP_SEND, flags: 0, ioprio: 0, fd: conn.fd, off: 0, addr: response.as_ptr() as u64, len: response.len() as u32, rw_flags: 0, user_data: (3u64 << 32) | conn_id as u64, pad: [0; 3], }; self.submit_sqe(sqe) }
    #[inline(always)] unsafe fn parse_request(buffer: &[u8]) -> &'static [u8] { if buffer.len() < 16 || &buffer[0..4] != b"GET " { return HTTP_404; } let path_end = buffer[4..].iter().position(|&b| b == b' ').unwrap_or(buffer.len() - 4) + 4; if &buffer[4..path_end] == b"/" { HTTP_200_HELLO } else { HTTP_404 } }
    unsafe fn close_connection(&mut self, conn_id: usize) { if let Some(conn) = self.connections.get_mut(conn_id) { if conn.active { libc::syscall(SYS_CLOSE, conn.fd); conn.active = false; conn.fd = -1; self.free_list.push(conn_id); } } }
    
    unsafe fn run(&mut self) -> ! {
        for _ in 0..(IO_URING_QUEUE_SIZE / 2) { self.submit_accept(); }
        loop {
            let to_submit = (*self.sq_tail).wrapping_sub(*self.sq_head);
            let ret = libc::syscall(SYS_IO_URING_ENTER, self.ring_fd, to_submit, 1, IORING_ENTER_GETEVENTS, 0 as *const c_void, 0);
            if ret < 0 { if ret == EINTR || ret == EAGAIN { continue; } libc::syscall(SYS_EXIT, -ret); }
            
            let mut head = *self.cq_head;
            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
            while head != core::ptr::read_volatile(self.cq_tail) {
                let cqe = &*self.cqes.add((head & self.cq_mask) as usize);
                let op_type = (cqe.user_data >> 32) as u32;
                let conn_id = (cqe.user_data & 0xFFFFFFFF) as usize;
                
                match op_type {
                    1 => { // Accept
                        self.submit_accept();
                        if cqe.res >= 0 { if let Some(new_conn_id) = self.free_list.pop() { let conn = &mut self.connections[new_conn_id]; conn.fd = cqe.res; conn.active = true; if !self.submit_recv(new_conn_id) { self.close_connection(new_conn_id); } } else { libc::syscall(SYS_CLOSE, cqe.res); } }
                    }
                    2 => { // Recv
                        if cqe.res > 0 { let buffer_len = cqe.res as usize; let response = Self::parse_request(&self.connections[conn_id].buffer[..buffer_len]); if !self.submit_send(conn_id, response) { self.close_connection(conn_id); } } else { self.close_connection(conn_id); }
                    }
                    3 => { // Send
                        if cqe.res > 0 { if !self.submit_recv(conn_id) { self.close_connection(conn_id); } } else { self.close_connection(conn_id); }
                    }
                    _ => {}
                }
                head = head.wrapping_add(1);
            }
            *self.cq_head = head;
            core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        }
    }
}

// Helper to print error codes
unsafe fn print_error(stage: &str, code: i64) {
    let stage_bytes = stage.as_bytes();
    let msg1 = b"Initialization failed at stage '";
    let msg2 = b"': error code ";
    libc::syscall(SYS_WRITE, 2, msg1.as_ptr() as *const c_void, msg1.len());
    libc::syscall(SYS_WRITE, 2, stage_bytes.as_ptr() as *const c_void, stage_bytes.len());
    libc::syscall(SYS_WRITE, 2, msg2.as_ptr() as *const c_void, msg2.len());
    
    let mut num = -code;
    let mut buf = [0u8; 20];
    let mut i = buf.len() - 1;
    if num == 0 { num = 1; } // Handle mmap's -1 error
    loop {
        buf[i] = (num % 10) as u8 + b'0';
        num /= 10;
        if num == 0 { break; }
        i -= 1;
    }
    libc::syscall(SYS_WRITE, 2, buf.as_ptr().add(i) as *const c_void, buf.len() - i);
    let newline = b"\n";
    libc::syscall(SYS_WRITE, 2, newline.as_ptr() as *const c_void, 1);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        match UltimateServer::new() {
            Ok(mut server) => {
                let msg = b"Server listening on port 8080\n";
                libc::syscall(SYS_WRITE, 1, msg.as_ptr() as *const c_void, msg.len());
                server.run();
            }
            Err((code, stage)) => {
                print_error(stage, code);
                libc::syscall(SYS_EXIT, 1);
                loop {}
            }
        }
    }
}

#[link(name = "c")]
extern "C" { fn syscall(num: i64, ...) -> i64; }

#[allow(non_camel_case_types)]
mod libc {
    pub use super::*;
    pub const AF_INET: i32 = 2; pub const SOCK_STREAM: i32 = 1; pub const SOL_SOCKET: i32 = 1;
    pub const SO_REUSEADDR: i32 = 2; pub const SO_REUSEPORT: i32 = 15; pub const PROT_READ: i32 = 1;
    pub const PROT_WRITE: i32 = 2; pub const MAP_SHARED: i32 = 1;
    pub const MAP_FAILED: *mut core::ffi::c_void = -1isize as *mut core::ffi::c_void;
    extern "C" {
        pub fn malloc(size: usize) -> *mut core::ffi::c_void;
        pub fn free(ptr: *mut core::ffi::c_void);
        pub fn mmap(addr: *mut core::ffi::c_void, length: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut core::ffi::c_void;
    }
}