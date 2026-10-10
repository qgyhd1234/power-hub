//! Windows USB 设备树枚举，把每个下游端口的设备映射到通道（ADR-0002）。
//!
//! 定位"本 hub"两级策略：
//! 1. 魔术字 VID/PID（`magic.rs`，拿到硬件后更新）；
//! 2. 未配置时退化用"下游设备数 = 7"特征匹配。
//!
//! 通道与物理端口的对应关系由硬件布线决定，当前按"层级 + 端口号"排序填入
//! 1~7——这也是拿到硬件后需要校准的魔术字性质信息。

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation as cfg;

use crate::magic;
use crate::protocol::CHANNELS;
use crate::state::DeviceInfo;

pub fn enumerate_channels() -> Result<[Option<DeviceInfo>; CHANNELS], String> {
    let ids = all_usb_device_ids();
    let hub = find_hub(&ids)?;
    let mut found: Vec<(u32, u32, DeviceInfo)> = Vec::new();
    collect(hub, 0, &mut found);
    found.sort_by_key(|(level, port, _)| (*level, *port));

    let mut out: [Option<DeviceInfo>; CHANNELS] = std::array::from_fn(|_| None);
    for (slot, (_, _, info)) in out.iter_mut().zip(found) {
        *slot = Some(info);
    }
    Ok(out)
}

fn find_hub(ids: &[String]) -> Result<u32, String> {
    if magic::HUB_VID != 0 || magic::HUB_PID != 0 {
        for id in ids {
            if let Some((vid, pid)) = parse_vid_pid(id) {
                if vid == magic::HUB_VID && pid == magic::HUB_PID {
                    if let Some(node) = locate(id) {
                        return Ok(node);
                    }
                }
            }
        }
    }
    // 特征匹配：某个有子设备的 USB 节点，其下游终端设备恰好 EXPECTED_PORTS 个
    for id in ids {
        if let Some(node) = locate(id) {
            if first_child(node).is_some() {
                let mut acc = Vec::new();
                collect(node, 0, &mut acc);
                if acc.len() == magic::EXPECTED_PORTS {
                    return Ok(node);
                }
            }
        }
    }
    Err("未找到 power-hub：魔术字未配置，且未发现下游 7 设备的 hub".into())
}

/// 深度优先收集 hub 下游的终端设备（跳过级联 hub 层级记为 level+1）。
fn collect(node: u32, level: u32, out: &mut Vec<(u32, u32, DeviceInfo)>) {
    let mut cur = first_child(node);
    while let Some(child) = cur {
        if let Some(id) = device_id(child) {
            if is_hub_node(child) {
                collect(child, level + 1, out);
            } else if let Some((vid, pid)) = parse_vid_pid(&id) {
                let port = parse_port(&id).unwrap_or(0);
                let name = reg_string(child, cfg::CM_DRP_DEVICEDESC)
                    .unwrap_or_else(|| "USB 设备".into());
                out.push((level, port, DeviceInfo { name, vid, pid }));
            }
        }
        cur = next_sibling(child);
    }
}

/// 有子节点、且子节点不是 MI_ 接口节点（复合设备的接口子节点不算 hub）。
fn is_hub_node(node: u32) -> bool {
    match first_child(node) {
        None => false,
        Some(g) => device_id(g).is_some_and(|id| !id.contains("&MI_")),
    }
}

fn all_usb_device_ids() -> Vec<String> {
    let filter = wide("USB");
    let mut size = 0u32;
    unsafe {
        let _ = cfg::CM_Get_Device_ID_List_SizeW(
            &mut size,
            PCWSTR(filter.as_ptr()),
            cfg::CM_GETIDLIST_FILTER_ENUMERATOR,
        );
    }
    if size == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u16; size as usize];
    let ret = unsafe {
        cfg::CM_Get_Device_ID_ListW(
            PCWSTR(filter.as_ptr()),
            &mut buf,
            cfg::CM_GETIDLIST_FILTER_ENUMERATOR,
        )
    };
    if ret != cfg::CR_SUCCESS {
        return Vec::new();
    }
    buf.split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf16_lossy(s))
        .collect()
}

fn locate(id: &str) -> Option<u32> {
    let mut w = wide(id);
    let mut node = 0u32;
    let ret = unsafe {
        cfg::CM_Locate_DevNodeW(&mut node, PWSTR(w.as_mut_ptr()), cfg::CM_LOCATE_DEVNODE_NORMAL)
    };
    (ret == cfg::CR_SUCCESS).then_some(node)
}

fn first_child(node: u32) -> Option<u32> {
    let mut child = 0u32;
    let ret = unsafe { cfg::CM_Get_Child(&mut child, node, 0) };
    (ret == cfg::CR_SUCCESS).then_some(child)
}

fn next_sibling(node: u32) -> Option<u32> {
    let mut sibling = 0u32;
    let ret = unsafe { cfg::CM_Get_Sibling(&mut sibling, node, 0) };
    (ret == cfg::CR_SUCCESS).then_some(sibling)
}

fn device_id(node: u32) -> Option<String> {
    let mut buf = [0u16; 512];
    let ret = unsafe { cfg::CM_Get_Device_IDW(node, &mut buf, 0) };
    (ret == cfg::CR_SUCCESS).then(|| from_wide(&buf))
}

fn reg_string(node: u32, prop: u32) -> Option<String> {
    let mut buf = [0u16; 512];
    let mut len = (buf.len() * 2) as u32;
    let ret = unsafe {
        cfg::CM_Get_DevNode_Registry_PropertyW(
            node,
            prop,
            None,
            Some(buf.as_mut_ptr().cast()),
            &mut len,
            0,
        )
    };
    if ret != cfg::CR_SUCCESS || len < 2 {
        return None;
    }
    Some(from_wide(&buf[..(len as usize / 2).min(buf.len())]))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// 从设备实例 ID 提取 (VID, PID)，如 `USB\VID_1A86&PID_3346\...`
fn parse_vid_pid(id: &str) -> Option<(u16, u16)> {
    let dev = id.split('\\').nth(1)?;
    let mut vid = None;
    let mut pid = None;
    for part in dev.split('&') {
        let p = part.to_ascii_uppercase();
        if let Some(v) = p.strip_prefix("VID_") {
            vid = u16::from_str_radix(v, 16).ok();
        }
        if let Some(v) = p.strip_prefix("PID_") {
            pid = u16::from_str_radix(v, 16).ok();
        }
    }
    Some((vid?, pid?))
}

/// 实例路径尾段 `...&<port>` 即父 hub 下的端口号。
fn parse_port(id: &str) -> Option<u32> {
    id.rsplit('&').next()?.parse().ok()
}
