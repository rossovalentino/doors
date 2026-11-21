//! cfgmgr32 exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

const CR_INVALID_POINTER: u32 = 0x03;
const CR_NO_SUCH_DEVNODE: u32 = 0x0D;
fn no_node(out: usize) -> u32 {
    if out == 0 {
        return CR_INVALID_POINTER;
    }
    // SAFETY: the caller's DEVINST out-parameter.
    unsafe { (out as *mut u32).write_unaligned(0) };
    CR_NO_SUCH_DEVNODE
}
#[unsafe(no_mangle)]
pub extern "system" fn CM_Locate_DevNodeW(node: usize, _id: usize, _flags: u32) -> u32 {
    no_node(node)
}
#[unsafe(no_mangle)]
pub extern "system" fn CM_Get_Parent(parent: usize, _node: u32, _flags: u32) -> u32 {
    no_node(parent)
}
#[unsafe(no_mangle)]
pub extern "system" fn CM_Get_Child(child: usize, _node: u32, _flags: u32) -> u32 {
    no_node(child)
}
#[unsafe(no_mangle)]
pub extern "system" fn CM_Get_Sibling(sibling: usize, _node: u32, _flags: u32) -> u32 {
    no_node(sibling)
}
#[unsafe(no_mangle)]
pub extern "system" fn CM_Get_DevNode_PropertyW(
    _node: u32,
    _key: usize,
    _kind: usize,
    _buffer: usize,
    _size: usize,
    _flags: u32,
) -> u32 {
    CR_NO_SUCH_DEVNODE
}
#[unsafe(no_mangle)]
pub extern "system" fn CM_Get_Device_IDW(
    _node: u32,
    _buffer: usize,
    _length: u32,
    _flags: u32,
) -> u32 {
    CR_NO_SUCH_DEVNODE
}
