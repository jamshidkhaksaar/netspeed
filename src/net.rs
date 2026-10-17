use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetBestInterface, GetIfTable2, MIB_IF_TABLE2};

/// IF_TYPE_SOFTWARE_LOOPBACK (24) from ipifcons.h
pub const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
/// IF_TYPE_TUNNEL (131)
pub const IF_TYPE_TUNNEL: u32 = 131;
/// IfOperStatusUp (1)
const IF_OPER_STATUS_UP: i32 = 1;

#[derive(Clone)]
pub struct AdapterRow {
    pub luid: u64,
    pub index: u32,
    pub alias: String,
    pub description: String,
    pub if_type: u32,
    pub up: bool,
    pub link_bps: u64,
    pub in_octets: u64,
    pub out_octets: u64,
}

impl AdapterRow {
    pub fn is_physical_like(&self) -> bool {
        if self.if_type == IF_TYPE_SOFTWARE_LOOPBACK || self.if_type == IF_TYPE_TUNNEL {
            return false;
        }
        let a = self.alias.to_lowercase();
        let d = self.description.to_lowercase();
        let filtery = |s: &str| {
            s.contains("wfp")
                || s.contains("filter")
                || s.contains("qos packet scheduler")
                || s.contains("hyper-v")
                || s.contains("vswitch")
                || s.contains("wan miniport")
        };
        !filtery(&a) && !filtery(&d)
    }
}

pub fn list_adapters() -> Vec<AdapterRow> {
    let mut rows = Vec::new();
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        let rc = GetIfTable2(&mut table);
        if rc != windows::Win32::Foundation::ERROR_SUCCESS || table.is_null() {
            return rows;
        }
        let t = &*table;
        let entries = t.Table.as_ptr();
        for i in 0..t.NumEntries as usize {
            let r = &*entries.add(i);
            rows.push(AdapterRow {
                luid: r.InterfaceLuid.Value,
                index: r.InterfaceIndex,
                alias: crate::util::from_wide_buf(&r.Alias),
                description: crate::util::from_wide_buf(&r.Description),
                if_type: r.Type as u32,
                up: (r.OperStatus.0) == IF_OPER_STATUS_UP,
                link_bps: r.ReceiveLinkSpeed,
                in_octets: r.InOctets,
                out_octets: r.OutOctets,
            });
        }
        FreeMibTable(table as *const _);
    }
    rows
}

/// Best-route interface index for destination 8.8.8.8 (network byte order 0x08080808).
pub fn best_route_index() -> Option<u32> {
    unsafe {
        let mut idx: u32 = 0;
        let rc = GetBestInterface(0x0808_0808, &mut idx);
        if rc == 0 && idx != 0 { Some(idx) } else { None }
    }
}
