use std::{path::PathBuf, sync::Arc};

use super::{
    fs::{BlockDevice, FileType},
    inode::*,
};

pub struct OpenFile {
    pub dev: Arc<dyn BlockDevice>,
    pub offset: u32,
    pub path: PathBuf,
    pub ip: Option<InodePtr>,
}

pub struct Stat {
    pub name: String,
    pub size: u32,
    pub type_: FileType,
}

pub fn fstat(file: &OpenFile) -> Stat {
    let mut stat = Stat {
        name: file.path.file_name().unwrap().to_str().unwrap().to_string(),
        size: 0,
        type_: FileType::Free,
    };
    if let Some(ip) = &file.ip {
        stat = ip.read_disk_inode(|diskinode| Stat {
            name: file
                .path
                .clone()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
            size: diskinode.size,
            type_: match diskinode.ftype {
                0 => FileType::Free,
                1 => FileType::File,
                2 => FileType::Dir,
                _ => panic!("unknown file type"),
            },
        })
    }
    stat
}

pub fn ls(dev: Arc<dyn BlockDevice>, file: &OpenFile) {
    let ip = file.ip.as_ref().unwrap();
}

pub fn fcreat(dev: Arc<dyn BlockDevice>, path: &PathBuf, type_: FileType) -> OpenFile {
    let inode = create(dev.clone(), path, type_);
    OpenFile {
        dev,
        offset: 0,
        path: path.clone(),
        ip: inode,
    }
}

pub fn funlink(dev: Arc<dyn BlockDevice>, path: &PathBuf) {
    let mut ip = find_inode(dev.clone(), path).unwrap();
    let mut dp = find_parent_inode(dev.clone(), path).unwrap();
    dirunlink(dev, &mut dp, path.file_name().unwrap().to_str().unwrap());
    ip.modify_disk_inode(|diskinode| {
        diskinode.nlink -= 1;
    })
}

pub fn fopen(dev: Arc<dyn BlockDevice>, path: &PathBuf) -> OpenFile {
    let inode = find_inode(dev.clone(), path);
    OpenFile {
        dev,
        offset: 0,
        path: path.clone(),
        ip: inode,
    }
}

pub fn fseek(file: &mut OpenFile, offset: u32) {
    file.offset = offset;
}

// pub fn fclose(mut file: OpenFile) {
//     fsync(&mut file);
// }

pub fn fread(file: &mut OpenFile, dst: &mut [u8]) -> usize {
    let n = rinode(
        file.ip.as_mut().unwrap(),
        dst,
        file.offset as usize,
        dst.len(),
    );
    file.offset += n as u32;
    n
}

pub fn fwrite(file: &mut OpenFile, src: &[u8]) -> usize {
    let n = winode(
        file.ip.as_mut().unwrap(),
        src,
        file.offset as usize,
        src.len(),
    );
    file.offset += n as u32;
    n
}

// pub fn fsync(file: &mut OpenFile) {
//     sync_all();
// }
