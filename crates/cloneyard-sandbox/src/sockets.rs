//! The seccomp filters: which sockets a clone opens, and no `io_uring`, which opens sockets without the syscall.

use std::collections::BTreeMap;

use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
    SeccompRule, TargetArch,
};

use crate::Error;

const SOCK_TYPE_MASK: u64 = 0xf;

/// Applies the filters to this process and the ones it starts; a syscall of another ABI, such as i386, kills the process.
pub(crate) fn filter() -> Result<(), Error> {
    let arch = TargetArch::try_from(std::env::consts::ARCH)?;
    let unix = [libc::SOCK_STREAM, libc::SOCK_SEQPACKET]
        .into_iter()
        .map(|kind| {
            SeccompRule::new(vec![
                SeccompCondition::new(
                    0,
                    SeccompCmpArgLen::Dword,
                    SeccompCmpOp::Eq,
                    u64::from(libc::AF_UNIX.unsigned_abs()),
                )?,
                SeccompCondition::new(
                    1,
                    SeccompCmpArgLen::Dword,
                    SeccompCmpOp::MaskedEq(SOCK_TYPE_MASK),
                    u64::from(kind.unsigned_abs()),
                )?,
            ])
        })
        .collect::<Result<Vec<_>, _>>()?;
    let sockets = numbers(libc::SYS_socket)
        .map(|number| (number, unix.clone()))
        .collect();
    let io_uring = [
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
    ]
    .into_iter()
    .flat_map(numbers)
    .map(|number| (number, Vec::new()))
    .collect();
    for (rules, errno) in [(sockets, libc::EACCES), (io_uring, libc::EPERM)] {
        apply(rules, errno, arch)?;
    }
    Ok(())
}

fn apply(
    rules: BTreeMap<i64, Vec<SeccompRule>>,
    errno: i32,
    arch: TargetArch,
) -> Result<(), Error> {
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(errno.unsigned_abs()),
        arch,
    )?;
    let program: BpfProgram = filter.try_into()?;
    seccompiler::apply_filter(&program)?;
    Ok(())
}

/// The numbers of syscall `number`: on `x86_64` also its x32 number, which has the same architecture in the filter.
fn numbers(number: i64) -> impl Iterator<Item = i64> {
    #[cfg(target_arch = "x86_64")]
    let x32 = Some(number | 0x4000_0000);
    #[cfg(not(target_arch = "x86_64"))]
    let x32 = None;
    std::iter::once(number).chain(x32)
}
