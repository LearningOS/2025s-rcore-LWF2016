//! File and filesystem-related syscalls
use crate::fs::inode::ROOT_INODE;
use crate::fs::{open_file, OSInode, OpenFlags, Stat};
use crate::mm::{translated_byte_buffer, translated_str, UserBuffer, VirtAddr, StepByOne};
use crate::task::{current_task, current_user_token};
use crate::config::PAGE_SIZE;
use alloc::sync::Arc;
use easy_fs::{block_cache_sync_all, DirEntry, DiskInode, Inode};
use easy_fs::DIRENT_SZ;

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        let l = file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize;
        l
    } else {
        -1
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap()) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        -1
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();
    0
}

/// YOUR JOB: Implement fstat.
pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    trace!(
        "kernel:pid[{}] sys_fstat IMPLEMENTED",
        current_task().unwrap().pid.0
    );

    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }

    let mut stat = Stat::new();
    stat.dev = 0;
    if let Some(file) = &inner.fd_table[fd]{
        if let Some(osinode) = file.as_any().downcast_ref::<OSInode>(){
            stat.mode = osinode.get_inode_mode();
            stat.ino = osinode.get_inode_id();
            stat.nlink = osinode.get_inode_nlink();
        }else{
            return -1;
        }
    }else{
        return -1;
    }

    let start = st as usize;
    let start_va = VirtAddr::from(start);
    
    let end = start + core::mem::size_of::<Stat>();
    let end_va = VirtAddr::from(end);
    
    
    let ptr = &stat as *const Stat as *const u8;
    let mut vpn = start_va.floor();
    let page_table = &mut inner.memory_set.page_table;
    if start_va.page_offset() <= PAGE_SIZE - core::mem::size_of::<Stat>(){
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

/// YOUR JOB: Implement linkat.
pub fn sys_linkat(old_name: *const u8, new_name: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_linkat IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let token = current_user_token();
    let old_name = translated_str(token, old_name);
    let new_name = translated_str(token, new_name);
    
    if let Some(old_inode) = ROOT_INODE.find(old_name.as_str()){
        ROOT_INODE.modify_disk_inode(|root_inode| {
            // append file in the dirent
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            // increase size
            ROOT_INODE.increase_size(new_size as u32, root_inode, &mut ROOT_INODE.fs.lock());
            // write dirent
            let dirent = DirEntry::new(new_name.as_str(), old_inode.get_inode_id() as u32);
            root_inode.write_at(
                file_count * DIRENT_SZ,
                dirent.as_bytes(),
                &ROOT_INODE.block_device,
            );
        });
        old_inode.add_nlink()
    }else{
        return -1;
    }
}

/// YOUR JOB: Implement unlinkat.
pub fn sys_unlinkat(name: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_unlinkat IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    // translate ptr to string
    let token = current_user_token();
    let name = translated_str(token, name);
    // lock the Easy File System
    let mut fs = ROOT_INODE.fs.lock();
    // find inode in ROOT INODE by name
    let inode = ROOT_INODE.read_disk_inode(|disk_inode| {
            ROOT_INODE.find_inode_id(name.as_str(), disk_inode).map(|inode_id| {
                let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
                Arc::new(Inode::new(
                    block_id,
                    block_offset,
                    ROOT_INODE.fs.clone(),
                    ROOT_INODE.block_device.clone(),
                ))
            })
        }).unwrap();
    // delete dirent in ROOT INODE
    ROOT_INODE.modify_disk_inode(|root_inode| {
        // count the file number
        let file_count = (root_inode.size as usize) / DIRENT_SZ;
        // find dirent
        let mut dirent = DirEntry::empty();
        for i in (0..file_count).into_iter(){
            root_inode.read_at(
                i * DIRENT_SZ,
                dirent.as_bytes_mut(),
                &ROOT_INODE.block_device,
            );
            if dirent.name() == name.as_str(){
                for j in  ((i + 1)..file_count).into_iter(){
                    let mut buf = DirEntry::new("\0", 0u32);
                    root_inode.read_at(
                        j*DIRENT_SZ, 
                        buf.as_bytes_mut(), 
                        &ROOT_INODE.block_device
                    );
                    root_inode.write_at(
                        (j-1)*DIRENT_SZ,
                        buf.as_bytes(), 
                        &ROOT_INODE.block_device
                    );
                }
                root_inode.size = ((file_count - 1) * DIRENT_SZ) as u32;
                break;
            }
        }
        // for i in (0..file_count+5).into_iter(){
        //     root_inode.read_at(
        //         i * DIRENT_SZ,
        //         dirent.as_bytes_mut(),
        //         &ROOT_INODE.block_device,
        //     );
        //     println!("{:?}", dirent.name());
        // }
    });
    // delete the disk inode or substract the nlink
    if inode.get_inode_nlink() > 1 {
        inode.modify_disk_inode(|disk_inode| {
                disk_inode.nlink -= 1;
            }); 
    }else{
        // problem in here
        inode.modify_disk_inode(|disk_inode| {
            disk_inode.nlink -= 1;
            let size = disk_inode.size;
            let data_blocks_dealloc = disk_inode.clear_size(&inode.block_device);
            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);
            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });
        let start_block = fs.get_inode_area_start_block();
        fs.dealloc_inode(((inode.block_id as u64 - start_block) * 4 + inode.block_offset as u64 / 128) as usize);
    }
    block_cache_sync_all();
    // println!("{}", inode.get_inode_nlink());
    0
}
