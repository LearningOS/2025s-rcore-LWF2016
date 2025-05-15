//! Process management syscalls
use crate::{config::PAGE_SIZE, mm::{PTEFlags, PageTable, StepByOne, VirtAddr}, task::{change_program_brk, current_user_token, exit_current_and_run_next, suspend_current_and_run_next, TASK_MANAGER}, timer::{get_time_ms, get_time_us}};
use crate::task::return_syscall_times;
#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let page_table = PageTable::from_token(current_user_token());
    
    let start = ts as usize;
    let start_va = VirtAddr::from(start);
    
    let end = start + core::mem::size_of::<TimeVal>();
    let end_va = VirtAddr::from(end);
    // println!("{}, {}", start, end);
    let time = TimeVal {sec: get_time_ms()/1000, usec: get_time_us()};
    
    let ptr = &time as *const TimeVal as *const u8;
    let mut vpn = start_va.floor();
    
    if start_va.page_offset() <= PAGE_SIZE - core::mem::size_of::<TimeVal>(){
        let ppn = page_table.translate(vpn).unwrap().ppn();
        for (i, offset) in (start_va.page_offset()..end_va.page_offset()).into_iter().enumerate(){
            unsafe{ppn.get_bytes_array()[offset] = *(ptr.add(i));}
        }
    }else{
        let first_ppn = page_table.translate(vpn).unwrap().ppn();
        vpn.step();
        let second_ppn = page_table.translate(vpn).unwrap().ppn();
        let first_mem = first_ppn.get_bytes_array();
        for (i, offset) in (start_va.page_offset()..PAGE_SIZE).into_iter().enumerate(){
            // println!("{:?}", start_va.page_offset());
            unsafe{first_mem[offset] = *(ptr.add(i));}
        }
        let second_mem = second_ppn.get_bytes_array();
        for (i, offset) in (0..end_va.page_offset()).into_iter().enumerate(){
            unsafe{second_mem[offset] = *(ptr.add(i + PAGE_SIZE - start_va.page_offset()));}
        }
    }
    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    
    let page_table = PageTable::from_token(current_user_token());
    let va = VirtAddr::from(id);
    let vpn = va.floor();
    match trace_request {
        0 => {
            if let Some(flag) = page_table.find_pte(vpn){
                if flag.flags() & PTEFlags::R == PTEFlags::empty() || flag.flags() & PTEFlags::U == PTEFlags::empty(){
                    -1
                }else{
                    let ppn = page_table.translate(vpn).unwrap().ppn();
                    ppn.get_bytes_array()[va.page_offset()] as isize
                }
            }else{
                -1
            }
        },
        1 => {
            if let Some(flag) = page_table.find_pte(vpn){
                if flag.flags() & PTEFlags::W == PTEFlags::empty() || flag.flags() & PTEFlags::U == PTEFlags::empty(){
                    -1
                }else{
                    let ppn = page_table.translate(vpn).unwrap().ppn();
                    ppn.get_bytes_array()[va.page_offset()] = data as u8;
                    0
                }
            }else{
                -1
            }
        },
        2 => return_syscall_times(id) as isize,
        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");
    TASK_MANAGER.map(start, len, prot)
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    TASK_MANAGER.unmap(start, len)
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
