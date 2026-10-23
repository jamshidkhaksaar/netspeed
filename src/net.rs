use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetBestInterface, GetIfTable2, MIB_IF_TABLE2,
};

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

/// A pin is exclusive: a missing or down pinned adapter never falls back.
/// Automatic selection prefers an up, non-loopback best route (including
/// VPNs/tunnels), then the fastest up physical-like interface. Equal link
/// speeds prefer the lowest interface index, then LUID, independent of row order.
/// Lifetime octet counters are deliberately not used to infer current activity.
pub fn pick_adapter(
    rows: &[AdapterRow],
    pinned_luid: Option<u64>,
    best_route_index: Option<u32>,
) -> Option<&AdapterRow> {
    if let Some(luid) = pinned_luid {
        return rows.iter().find(|row| row.luid == luid && row.up);
    }

    if let Some(index) = best_route_index {
        if let Some(row) = rows
            .iter()
            .find(|row| row.index == index && row.up && row.if_type != IF_TYPE_SOFTWARE_LOOPBACK)
        {
            return Some(row);
        }
    }

    rows.iter()
        .filter(|row| row.up && row.is_physical_like())
        .max_by_key(|row| {
            (
                row.link_bps,
                std::cmp::Reverse(row.index),
                std::cmp::Reverse(row.luid),
            )
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(luid: u64, index: u32, link_bps: u64) -> AdapterRow {
        AdapterRow {
            luid,
            index,
            alias: format!("Ethernet {index}"),
            description: String::from("Ethernet adapter"),
            if_type: 6,
            up: true,
            link_bps,
            in_octets: 0,
            out_octets: 0,
        }
    }

    #[test]
    fn pinned_adapter_overrides_route_and_faster_fallback() {
        let rows = [adapter(10, 1, 100), adapter(20, 2, 1_000)];
        let picked = pick_adapter(&rows, Some(10), Some(2)).unwrap();
        assert!(std::ptr::eq(picked, &rows[0]));
    }

    #[test]
    fn pinned_nonphysical_adapter_is_allowed() {
        let mut rows = [adapter(10, 1, 100), adapter(20, 2, 1_000)];
        rows[0].if_type = IF_TYPE_TUNNEL;
        rows[0].description = String::from("WAN Miniport (IKEv2)");
        assert!(!rows[0].is_physical_like());
        assert_eq!(
            pick_adapter(&rows, Some(10), Some(2)).map(|r| r.luid),
            Some(10)
        );
    }

    #[test]
    fn missing_pin_does_not_fall_back() {
        let rows = [adapter(10, 1, 100), adapter(20, 2, 1_000)];
        assert!(pick_adapter(&rows, Some(99), Some(2)).is_none());
    }

    #[test]
    fn down_pin_does_not_fall_back() {
        let mut rows = [adapter(10, 1, 10_000), adapter(20, 2, 1_000)];
        rows[0].up = false;
        rows[0].in_octets = u64::MAX;
        rows[0].out_octets = u64::MAX;
        assert!(pick_adapter(&rows, Some(10), Some(2)).is_none());
    }

    #[test]
    fn automatic_prefers_active_best_route_over_faster_link() {
        let rows = [adapter(10, 1, 100), adapter(20, 2, 1_000)];
        assert_eq!(pick_adapter(&rows, None, Some(1)).map(|r| r.luid), Some(10));
    }

    #[test]
    fn automatic_accepts_best_route_vpn() {
        let mut rows = [adapter(10, 1, 1_000), adapter(20, 2, 0)];
        rows[1].if_type = IF_TYPE_TUNNEL;
        rows[1].alias = String::from("VPN");
        rows[1].description = String::from("WAN Miniport (IKEv2)");
        assert!(!rows[1].is_physical_like());
        assert_eq!(pick_adapter(&rows, None, Some(2)).map(|r| r.luid), Some(20));
    }

    #[test]
    fn automatic_down_route_uses_fallback() {
        let mut rows = [adapter(10, 1, 100), adapter(20, 2, 10_000)];
        rows[1].up = false;
        rows[1].in_octets = u64::MAX;
        rows[1].out_octets = u64::MAX;
        assert_eq!(pick_adapter(&rows, None, Some(2)).map(|r| r.luid), Some(10));
    }

    #[test]
    fn automatic_loopback_route_uses_fallback() {
        let mut rows = [adapter(10, 1, 100), adapter(20, 2, 10_000)];
        rows[1].if_type = IF_TYPE_SOFTWARE_LOOPBACK;
        assert_eq!(pick_adapter(&rows, None, Some(2)).map(|r| r.luid), Some(10));
    }

    #[test]
    fn automatic_missing_route_uses_fallback() {
        let rows = [adapter(10, 1, 100), adapter(20, 2, 1_000)];
        assert_eq!(
            pick_adapter(&rows, None, Some(99)).map(|r| r.luid),
            Some(20)
        );
    }

    #[test]
    fn automatic_fallback_uses_link_speed_not_cumulative_octets() {
        let mut rows = [adapter(10, 1, 100), adapter(20, 2, 1_000)];
        rows[0].in_octets = u64::MAX;
        rows[0].out_octets = u64::MAX;
        rows[1].if_type = 71;
        rows[1].alias = String::from("Wi-Fi");
        assert_eq!(pick_adapter(&rows, None, None).map(|r| r.luid), Some(20));
    }

    #[test]
    fn automatic_fallback_accepts_idle_up_adapter_with_unknown_link_speed() {
        let rows = [adapter(10, 1, 0)];
        assert_eq!(pick_adapter(&rows, None, None).map(|r| r.luid), Some(10));
    }

    #[test]
    fn automatic_fallback_ignores_down_virtual_and_filtered_adapters() {
        let mut rows = [
            adapter(10, 1, 100),
            adapter(20, 2, 10_000),
            adapter(30, 3, 10_000),
            adapter(40, 4, 10_000),
            adapter(50, 5, 10_000),
            adapter(60, 6, 10_000),
            adapter(70, 7, 10_000),
        ];
        rows[1].up = false;
        rows[2].if_type = IF_TYPE_SOFTWARE_LOOPBACK;
        rows[3].if_type = IF_TYPE_TUNNEL;
        rows[4].alias = String::from("WFP filter");
        rows[5].description = String::from("Hyper-V Virtual Ethernet Adapter");
        rows[6].alias = String::from("WAN Miniport (PPPOE)");
        assert_eq!(pick_adapter(&rows, None, None).map(|r| r.luid), Some(10));
    }

    #[test]
    fn automatic_fallback_ties_are_order_independent() {
        let mut rows = vec![
            adapter(10, 8, 1_000),
            adapter(30, 2, 1_000),
            adapter(20, 2, 1_000),
        ];
        assert_eq!(pick_adapter(&rows, None, None).map(|r| r.luid), Some(20));
        rows.reverse();
        assert_eq!(pick_adapter(&rows, None, None).map(|r| r.luid), Some(20));
    }

    #[test]
    fn disconnected_adapters_return_none() {
        let mut rows = [adapter(10, 1, 100), adapter(20, 2, 1_000)];
        for row in &mut rows {
            row.up = false;
            row.in_octets = u64::MAX;
            row.out_octets = u64::MAX;
        }
        assert!(pick_adapter(&rows, None, None).is_none());
        assert!(pick_adapter(&rows, None, Some(2)).is_none());
    }

    #[test]
    fn automatic_without_eligible_fallback_returns_none() {
        let mut rows = [
            adapter(10, 1, 100),
            adapter(20, 2, 1_000),
            adapter(30, 3, 1_000),
            adapter(40, 4, 1_000),
        ];
        rows[0].up = false;
        rows[1].if_type = IF_TYPE_TUNNEL;
        rows[2].if_type = IF_TYPE_SOFTWARE_LOOPBACK;
        rows[3].description = String::from("QoS Packet Scheduler");
        assert!(pick_adapter(&rows, None, None).is_none());
        assert!(pick_adapter(&rows, None, Some(99)).is_none());
        assert!(pick_adapter(&rows, None, Some(3)).is_none());
    }

    #[test]
    fn empty_rows_return_none() {
        assert!(pick_adapter(&[], None, None).is_none());
        assert!(pick_adapter(&[], None, Some(1)).is_none());
        assert!(pick_adapter(&[], Some(10), Some(1)).is_none());
    }
}
