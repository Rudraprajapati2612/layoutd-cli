#[account(zero_copy)]
#[repr(C)]
pub struct VaultZC {
    pub owner:   Pubkey,
    pub balance: u64,
    pub bump:    u8,
}
