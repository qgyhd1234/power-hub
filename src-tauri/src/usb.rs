//! Windows USB 设备树枚举，把每个下游端口的设备映射到通道（ADR-0002）。
//!
//! 定位"本 hub"两级策略：
//! 1. 魔术字 VID/PID（`magic.rs`，实测 TUSB8041 两片级联，取最上游一片）；
//! 2. 未配置时退化用"下游设备数 = 7"特征匹配。
//!
//! 通道与物理端口的对应关系由硬件布线决定，当前按"层级 + 端口号"排序填入
//! 1~7；板载 CH340 控制串口在枚举时排除（见 collect），实机联调时仍需按
//! §10 验收矩阵校准用户设备与通道的映射。

use log::{debug, info};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation as cfg;
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_Address, DEVPKEY_Device_BusReportedDeviceDesc, DEVPROP_TYPE_STRING,
    DEVPROP_TYPE_UINT32, DEVPROPTYPE,
};

use crate::magic;
use crate::protocol::CHANNELS;
use crate::state::DeviceInfo;

pub fn enumerate_channels() -> Result<[Option<DeviceInfo>; CHANNELS], String> {
    let ids = all_usb_device_ids();
    let roots = find_hub_roots(&ids)?;
    let mut found: Vec<(u32, u32, DeviceInfo)> = Vec::new();
    // 每片物理 hub 在系统里有 HS/SS 两个平行节点，根集合可能不止一个
    for root in &roots {
        collect(*root, 0, &mut found);
    }
    found.sort_by_key(|(level, port, _)| (*level, *port));
    info!("设备树枚举: {} 个根节点, {} 个终端设备", roots.len(), found.len());
    for (level, port, d) in &found {
        debug!("设备树: level={level} port={port} {} {:04X}:{:04X}", d.name, d.vid, d.pid);
    }

    let mut out: [Option<DeviceInfo>; CHANNELS] = std::array::from_fn(|_| None);
    for (slot, (_, _, info)) in out.iter_mut().zip(found) {
        *slot = Some(info);
    }
    Ok(out)
}

/// 定位"本 hub"：按 VID 收集所有最上游的 hub 半边节点（每片物理 hub 有
/// HS/SS 两个并联 hub 节点、PID 不同但 VID 相同；且两半的父节点不同——
/// 分别挂在板外上游 hub 的 USB2 半与 SS 半，故只能按 VID 全扫再筛最上游）。
fn find_hub_roots(ids: &[String]) -> Result<Vec<u32>, String> {
    let mut roots = Vec::new();
    if magic::HUB_VID != 0 {
        for id in ids {
            if let Some((vid, _)) = parse_vid_pid(id) {
                if vid != magic::HUB_VID {
                    continue;
                }
                if let Some(node) = locate(id) {
                    // 排除级联下游片（父也是本 hub）；须真是 hub，避免 TI 其它设备误收
                    if !parent_matches_hub(node) && is_hub_node(node) {
                        roots.push(node);
                    }
                }
            }
        }
    }
    roots.sort_unstable();
    if !roots.is_empty() {
        return Ok(roots);
    }
    // 特征匹配：某个有子设备的 USB 节点，其下游终端设备恰好 EXPECTED_PORTS 个
    for id in ids {
        if let Some(node) = locate(id) {
            if first_child(node).is_some() {
                let mut acc = Vec::new();
                collect(node, 0, &mut acc);
                if acc.len() == magic::EXPECTED_PORTS {
                    return Ok(vec![node]);
                }
            }
        }
    }
    Err("未找到 power-hub：魔术字未配置，且未发现下游 7 设备的 hub".into())
}

/// 父节点是否仍是同 VID 的 hub —— 是则当前节点为级联下游片，不是锚点。
/// （HS/SS 半边 PID 不同，不能按 PID 比）
fn parent_matches_hub(node: u32) -> bool {
    let mut parent = 0u32;
    if unsafe { cfg::CM_Get_Parent(&mut parent, node, 0) } != cfg::CR_SUCCESS {
        return false;
    }
    device_id(parent)
        .and_then(|id| parse_vid_pid(&id))
        .is_some_and(|(vid, _)| vid == magic::HUB_VID)
}

/// 深度优先收集 hub 下游的终端设备（跳过级联 hub 层级记为 level+1）。
fn collect(node: u32, level: u32, out: &mut Vec<(u32, u32, DeviceInfo)>) {
    let mut cur = first_child(node);
    while let Some(child) = cur {
        if let Some(id) = device_id(child) {
            if is_hub_node(child) {
                collect(child, level + 1, out);
            } else if let Some((vid, pid)) = parse_vid_pid(&id) {
                // 板载控制串口芯片不算用户设备：不进设备数、不占通道
                let is_control =
                    vid == crate::magic::CONTROL_VID && pid == crate::magic::CONTROL_PID;
                if !is_control {
                    let port = device_port(child, &id);
                    let name = device_name(child).unwrap_or_else(|| "USB 设备".into());
                    out.push((level, port, DeviceInfo { name, vid, pid }));
                }
            }
        }
        cur = next_sibling(child);
    }
}

/// 设备名（USB Tree View 风格，尽量"型号 + COM 号"）：
/// 1. FriendlyName 比 DeviceDesc 更具体时用它——Ports 类设备被系统追加了
///    " (COMx)" 后缀（如 "USB-Enhanced-SERIAL CH9102 (COM34)"），是区分
///    同厂多个串口芯片的关键；固件上报名对串口芯片常是 "USB Serial" 泛词；
/// 2. 否则用固件上报的产品描述（多数设备即产品名，如 "MIIIW 2.4G Receiver"）；
/// 3. 兜底注册表设备描述。
fn device_name(node: u32) -> Option<String> {
    let friendly = reg_string(node, cfg::CM_DRP_FRIENDLYNAME);
    let desc = reg_string(node, cfg::CM_DRP_DEVICEDESC);
    if let Some(f) = friendly.filter(|s| !s.trim().is_empty()) {
        if desc.as_deref() != Some(f.as_str()) {
            return Some(f);
        }
    }
    devnode_string(node, &DEVPKEY_Device_BusReportedDeviceDesc)
        .filter(|s| !s.trim().is_empty())
        .or(desc)
}

/// `CM_Get_DevNode_PropertyW` 读 REG_SZ 属性（两段式：先取长度再取内容）。
fn devnode_string(node: u32, key: &windows::Win32::Devices::Properties::DEVPROPKEY) -> Option<String> {
    let mut ptype = DEVPROPTYPE(0);
    let mut size = 0u32;
    unsafe { cfg::CM_Get_DevNode_PropertyW(node, key, &mut ptype, None, &mut size, 0) };
    if size < 2 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    let ret =
        unsafe { cfg::CM_Get_DevNode_PropertyW(node, key, &mut ptype, Some(buf.as_mut_ptr()), &mut size, 0) };
    if ret != cfg::CR_SUCCESS || ptype != DEVPROP_TYPE_STRING {
        return None;
    }
    let units: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
    if end == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&units[..end]))
}

/// 是 USB 集线器节点：按驱动服务判定（USBHUB3/USBHUB）。
/// "有子节点且子节点非 MI_" 的旧启发式会把 U 盘（子节点是 SCSI 功能设备）
/// 误判成 hub 递归进去导致漏收；复合设备（usbccgp）、U 盘（USBSTOR）在此判据下都不是 hub。
fn is_hub_node(node: u32) -> bool {
    let svc = reg_string(node, cfg::CM_DRP_SERVICE).map(|s| s.to_ascii_uppercase());
    matches!(svc.as_deref(), Some("USBHUB3") | Some("USBHUB"))
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

/// USB 设备的父 hub 端口号：优先 `DEVPKEY_Device_Address`（USB 设备即父端口，
/// 有序列号设备实例路径尾段是序列号而非端口号），失败回退实例尾段解析。
fn device_port(node: u32, id: &str) -> u32 {
    let mut ptype = DEVPROPTYPE(0);
    let mut size = 0u32;
    unsafe { cfg::CM_Get_DevNode_PropertyW(node, &DEVPKEY_Device_Address, &mut ptype, None, &mut size, 0) };
    if size == 4 {
        let mut buf = [0u8; 4];
        let ret = unsafe {
            cfg::CM_Get_DevNode_PropertyW(node, &DEVPKEY_Device_Address, &mut ptype, Some(buf.as_mut_ptr()), &mut size, 0)
        };
        if ret == cfg::CR_SUCCESS && ptype == DEVPROP_TYPE_UINT32 {
            return u32::from_le_bytes(buf);
        }
    }
    parse_port(id).unwrap_or(0)
}

/// 实例路径尾段 `...&<port>` 即父 hub 下的端口号（回退用）。
fn parse_port(id: &str) -> Option<u32> {
    id.rsplit('&').next()?.parse().ok()
}
