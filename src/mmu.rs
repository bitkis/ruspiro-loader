/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # MMU maintenance
//!
use ruspiro_register::system::*;

#[repr(align(4096))]
struct MmuConfig {
    ttlb_lvl0: [u64; 512],
    ttlb_lvl1: [u64; 513],
}

/// level 0 translation table, each entry covering 1GB of memory
/// level 1 translation table, each entry covering 2MB of memory
static mut MMU_CFG: MmuConfig = MmuConfig {
    ttlb_lvl0: [0; 512],
    ttlb_lvl1: [0; 513],
};

pub fn initialize_mmu(core: u32) {
    // disable MMU before any configuration changes happen
    disable_mmu();

    // setup ttlb entries - this is only needed once on the main core
    // as all cores share the same physical memory
    if core == 0 {
        setup_page_tables();
    }

    // configure the MAIR (memory attribute) variations we will support
    // those entries are referred to as index in the memeory attributes of the
    // table entries
    mair_el2::write(
        mair_el2::MAIR0::NGNRNE
            | mair_el2::MAIR1::NGNRE
            | mair_el2::MAIR2::GRE
            | mair_el2::MAIR3::NC
            | mair_el2::MAIR4::NORM,
    );

    // set the ttlb base address, this is where the memory address translation
    // table walk starts
    let ttlb_base = unsafe { (&MMU_CFG.ttlb_lvl0[0] as *const u64) as u64 };
    ttbr0_el2::write(ttbr0_el2::baddr::with_value(ttlb_base));

    // configure the TTLB attributes
    tcr_el2::write(
        tcr_el2::T0SZ::with_value(25)
            | tcr_el2::IRGN0::NM_IWB_RA_WA
            | tcr_el2::ORGN0::NM_OWB_RA_WA
            | tcr_el2::SH0::IS
            | tcr_el2::TG0::_4KB
            | tcr_el2::PS::_32BITS
            | tcr_el2::TBI::IGNORE,
    );

    hcr_el2::write(hcr_el2::DC::DISABLE | hcr_el2::VM::DISABLE);

    // set the SCTRL_EL2 to activate the MMU, as part of this the data cache is also enabled
    sctlr_el2::write(
        sctlr_el2::M::ENABLE
            | sctlr_el2::A::DISABLE
            | sctlr_el2::C::ENABLE
            | sctlr_el2::SA::DISABLE
            | sctlr_el2::I::DISABLE,
    );

    // let 2 cycles pass with a nop to settle the MMU
    nop();
    nop();
}

pub fn disable_mmu() {
    // disabling the MMU will also disable data and instruction cache
    sctlr_el2::write(sctlr_el2::M::DISABLE | sctlr_el2::C::DISABLE | sctlr_el2::I::DISABLE);
    // let 2 cycles pass with a nop to settle the MMU
    nop();
    nop();
    // as we have switched of the MMU we might also invalidate all TTLB entries
    unsafe {
        llvm_asm!(
            "dsb sy                   
             isb"
        )
    };
}

/// # Safety
/// A call to this initial MMU setup and configuration should always be called only once and from
/// the main core booting up first only. As long as the MMU is not up and running there is no way
/// to secure access with atmic operations as they require the MMU to not hang the core
fn setup_page_tables() {
    // initial MMU page table setup
    // this first attempt provides very huge configuration blocks, meaning we
    // setup the smallest unit to cover 2Mb blocks of memory sharing the same memory attributes
    unsafe {
        let level1_addr_1 = &MMU_CFG.ttlb_lvl1[0] as *const u64;
        let level1_addr_2 = &MMU_CFG.ttlb_lvl1[512] as *const u64;

        // the entries in level 0 (covering 1GB each) need to point to the next level table
        // that contains more granular config
        MMU_CFG.ttlb_lvl0[0] = 0x1 << 63 | (level1_addr_1 as u64) | 0b11;
        MMU_CFG.ttlb_lvl0[1] = 0x1 << 63 | (level1_addr_2 as u64) | 0b11;

        // the entries in level 1 (covering 2MB each) contain the specific memory attributes for
        // this memory area
        // first entries up to 0x3F00_0000 are "normal" memory
        for i in 0..504 {
            // 1:1 memory mapping with it's attributes
            // AF = 1 << 10, SH = 3 << 8, MAIR index = 4 << 2
            MMU_CFG.ttlb_lvl1[i] = (i as u64 * 0x20_0000) | 0x710 | 0b01;
        }

        // entries from 0x3F00_0000 to 0x4020_0000 are "device" memory
        for i in 504..513 {
            // 1:1 memory mapping with it's attributes
            // AF = 1 << 10, SH = 0 << 8, MAIR index = 0 << 2
            MMU_CFG.ttlb_lvl1[i] = (i as u64 * 0x20_0000) | 0x400 | 0b01;
        }

        llvm_asm!(
            "dsb   ishst
             tlbi  alle2is"
        );
    }
}
