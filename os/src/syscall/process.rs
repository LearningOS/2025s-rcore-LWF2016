//! Process management syscalls
//!
use alloc::sync::Arc;
use crate::config::PAGE_SIZE;

use crate::{
    fs::{open_file, File, OpenFlags},
    mm::{translated_refmut, translated_str, frame_alloc, PTEFlags, MapType, VirtAddr, StepByOne, MapPermission, PhysPageNum, PageTableEntry, MapArea, VirtPageNum},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next, TaskControlBlock
    },
    timer::{get_time_ms, get_time_us},
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let task = current_task().unwrap();
        task.exec(all_data.as_slice());
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    //trace!("kernel: sys_waitpid");
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_get_time IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let page_table = &mut inner.memory_set.page_table;
    
    let start = ts as usize;
    let start_va = VirtAddr::from(start);
    
    let end = start + core::mem::size_of::<TimeVal>();
    let end_va = VirtAddr::from(end);
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
            unsafe{first_mem[offset] = *(ptr.add(i));}
        }
        let second_mem = second_ppn.get_bytes_array();
        for (i, offset) in (0..end_va.page_offset()).into_iter().enumerate(){
            unsafe{second_mem[offset] = *(ptr.add(i + PAGE_SIZE - start_va.page_offset()));}
        }
    }
    0
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_mmap IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let task = current_task().unwrap();
    if prot & !0x7 != 0 || prot & 0x7 == 0{
        return -1
    }

    if start % PAGE_SIZE != 0 {
        return -1
    }

    let mut area = MapArea::new(VirtAddr::from(start), VirtAddr::from(start+len), MapType::Framed, MapPermission::from_bits((prot << 1) as u8).unwrap());
    let inner = &mut task.inner_exclusive_access();
    let page_table = &mut inner.memory_set.page_table;
    for vpn in area.vpn_range{
        let pte = page_table.find_pte_create(vpn).unwrap();
        if pte.is_valid(){
            return -1
        }
    }

    for vpn in area.vpn_range {
        let ppn: PhysPageNum;
        let frame = frame_alloc().unwrap();
        ppn = frame.ppn;
        area.data_frames.insert(vpn, frame);
        let pte_flags = PTEFlags::from_bits(((prot + 8) << 1) as u8).unwrap();
        let pte = page_table.find_pte_create(vpn).unwrap();
        *pte = PageTableEntry::new(ppn, pte_flags | PTEFlags::V);
    }

    inner.memory_set.areas.push(area);
    0
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_munmap IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let task = current_task().unwrap();
    let mem_set = &mut task.inner_exclusive_access().memory_set;
    let page_table = &mut mem_set.page_table;

    if start % PAGE_SIZE != 0 {
        return -1
    }

    for i in ((start/PAGE_SIZE)..=((start+len-1)/PAGE_SIZE)).into_iter(){
        let pte = page_table.find_pte(VirtPageNum::from(i)).unwrap();
        if !pte.is_valid(){
            return -1
        }
    }

    for i in ((start/PAGE_SIZE)..=((start+len-1)/PAGE_SIZE)).into_iter(){
        for area in &mut mem_set.areas{
            if VirtPageNum::from(i) < area.vpn_range.get_end() && VirtPageNum::from(i) >= area.vpn_range.get_start(){
                area.data_frames.remove(&VirtPageNum::from(i));
                page_table.unmap(VirtPageNum::from(i));
            }
        }
    }
    0
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(path: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_spawn IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(os_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        if !os_inode.readable(){
            return -1;
        }
        let task = Arc::new(TaskControlBlock::new(os_inode.read_all().as_slice()));
        let pid = task.pid.0;
        let trap_cx = task.inner_exclusive_access().get_trap_cx();
        trap_cx.x[10] = 0;
        let parent_inner = current_task().unwrap();
        let parent = &mut task.inner_exclusive_access().parent;
        *parent = Some(Arc::downgrade(&parent_inner));
        parent_inner.inner_exclusive_access().children.push(task.clone());

        add_task(task.clone());
        
        pid as isize
    } else {
        -1
    }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(_prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    -1
}
