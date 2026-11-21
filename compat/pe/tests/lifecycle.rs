mod support;
use door_pe_loader::lifecycle::{Error, Lifecycle, Phase, Reason};
use door_pe_loader::{BundlePlan, Feature, LoadPolicy, Reason as PlanReason};
use support::*;

fn images() -> (Image, Image, Image) {
    let mut exe = Image::new(false);
    exe.imports("outer.dll", &["Run"]);
    let mut outer = Image::new(true);
    outer.put32(OPTIONAL + 16, 0x1000);
    outer.exports(&[("Run", 0x1000)]);
    outer.imports("inner.dll", &["Run"]);
    let mut inner = Image::new(true);
    inner.put32(OPTIONAL + 16, 0x1000);
    inner.exports(&[("Run", 0x1000)]);
    (exe, outer, inner)
}

#[test]
fn callbacks_attach_dependency_first_and_detach_reverse_without_exe() {
    let (exe, outer, inner) = images();
    let sources = [
        exe.source("app.exe", EXE_BASE),
        outer.source("outer.dll", DLL_BASE),
        inner.source("inner.dll", DLL_BASE + 0x10000),
    ];
    assert_eq!(
        BundlePlan::new(&sources).unwrap_err().reason,
        PlanReason::Unsupported(Feature::DllInitialization)
    );
    let plan = BundlePlan::with_policy(&sources, LoadPolicy::DllCallbacks).unwrap();
    let mut state = Lifecycle::new(&plan);
    let mut old = None;
    for module in [2, 1] {
        let call = state.next_callback().unwrap().unwrap();
        assert_eq!(call.module, module);
        assert_eq!(call.reason, Reason::ProcessAttach);
        assert_eq!(state.next_callback(), Err(Error::CallbackPending));
        assert_eq!(state.begin_detach(), Err(Error::CallbackPending));
        assert!(!state.may_unmap());
        if let Some(stale) = old {
            assert_eq!(state.complete(stale, true), Err(Error::InvalidCompletion));
        }
        state.complete(call, true).unwrap();
        assert_eq!(state.complete(call, true), Err(Error::InvalidCompletion));
        old = Some(call);
    }
    assert_eq!(state.next_callback(), Ok(None));
    assert_eq!(state.phase(), Phase::Live);
    assert!(!state.may_unmap());
    state.begin_detach().unwrap();
    for module in [1, 2] {
        let call = state.next_callback().unwrap().unwrap();
        assert_eq!(call.module, module);
        assert_eq!(call.reason, Reason::ProcessDetach);
        state.complete(call, false).unwrap(); // detach return ignored
    }
    assert_eq!(state.next_callback(), Ok(None));
    assert_eq!(state.phase(), Phase::Unloaded);
    assert!(state.may_unmap());
    assert_eq!(state.begin_detach(), Err(Error::NotLive));
}

#[test]
fn failure_at_each_attach_detaches_failed_then_successfully_initialized_modules() {
    let (exe, outer, inner) = images();
    let plan = BundlePlan::with_policy(
        &[
            exe.source("app.exe", EXE_BASE),
            outer.source("outer.dll", DLL_BASE),
            inner.source("inner.dll", DLL_BASE + 0x10000),
        ],
        LoadPolicy::DllCallbacks,
    )
    .unwrap();
    for failed in 0..2 {
        let mut state = Lifecycle::new(&plan);
        let mut initialized = Vec::new();
        for step in 0..=failed {
            let call = state.next_callback().unwrap().unwrap();
            state.complete(call, step != failed).unwrap();
            if step != failed {
                initialized.push(call.module);
            } else {
                assert_eq!(state.failed_module(), Some(call.module));
                initialized.push(call.module);
            }
        }
        initialized.reverse();
        for module in initialized {
            let call = state.next_callback().unwrap().unwrap();
            assert_eq!(call.module, module);
            assert_eq!(call.reason, Reason::ProcessDetach);
            assert!(!call.reserved_nonnull);
            state.complete(call, true).unwrap();
        }
        assert_eq!(state.next_callback(), Ok(None));
        assert_eq!(state.phase(), Phase::Failed);
        assert!(state.may_unmap());
    }
}

#[test]
fn faulting_attach_is_not_called_again_and_leases_keep_mappings_alive() {
    let (exe, outer, inner) = images();
    let plan = BundlePlan::with_policy(
        &[
            exe.source("app.exe", EXE_BASE),
            outer.source("outer.dll", DLL_BASE),
            inner.source("inner.dll", DLL_BASE + 0x10000),
        ],
        LoadPolicy::DllCallbacks,
    )
    .unwrap();
    for failure in 0..2 {
        let mut state = Lifecycle::new(&plan);
        for _ in 0..failure {
            let call = state.next_callback().unwrap().unwrap();
            assert!(call.reserved_nonnull);
            state.complete(call, true).unwrap();
        }
        let fault = state.next_callback().unwrap().unwrap();
        assert!(!state.may_unmap());
        state.complete_fault(fault).unwrap();
        assert_eq!(state.complete_fault(fault), Err(Error::InvalidCompletion));
        let mut detached = 0;
        while let Some(call) = state.next_callback().unwrap() {
            assert_ne!(call.module, fault.module);
            assert!(!call.reserved_nonnull);
            assert!(!state.may_unmap());
            state.complete(call, true).unwrap();
            detached += 1;
        }
        assert_eq!(detached, failure);
        assert_eq!(state.phase(), Phase::Failed);
        assert!(state.may_unmap());
    }
}

#[test]
fn noentry_dependencies_require_no_callback_and_remain_owned_until_detach() {
    let (exe, mut outer, mut inner) = images();
    outer.put32(OPTIONAL + 16, 0);
    inner.put32(OPTIONAL + 16, 0);
    let plan = BundlePlan::new(&[
        exe.source("app.exe", EXE_BASE),
        outer.source("outer.dll", DLL_BASE),
        inner.source("inner.dll", DLL_BASE + 0x10000),
    ])
    .unwrap();
    let mut state = Lifecycle::new(&plan);
    assert_eq!(state.begin_detach(), Err(Error::NotLive));
    assert_eq!(state.next_callback(), Ok(None));
    assert_eq!(state.phase(), Phase::Live);
    state.begin_detach().unwrap();
    assert_eq!(state.next_callback(), Ok(None));
    assert!(state.may_unmap());
}

#[test]
fn callback_policy_rejects_noncode_entries_and_malformed_tls() {
    let (exe, mut outer, inner) = images();
    for rva in [0x200, 0x1200, DATA, 0x4000] {
        outer.put32(OPTIONAL + 16, rva);
        assert!(
            BundlePlan::with_policy(
                &[
                    exe.source("app.exe", EXE_BASE),
                    outer.source("outer.dll", DLL_BASE),
                    inner.source("inner.dll", DLL_BASE + 0x10000)
                ],
                LoadPolicy::DllCallbacks
            )
            .is_err()
        );
    }
    outer.put32(OPTIONAL + 16, 0x1000);
    outer.directory(9, 0x2800, 40);
    assert_eq!(
        BundlePlan::with_policy(
            &[
                exe.source("app.exe", EXE_BASE),
                outer.source("outer.dll", DLL_BASE),
                inner.source("inner.dll", DLL_BASE + 0x10000)
            ],
            LoadPolicy::DllCallbacks
        )
        .unwrap_err()
        .reason,
        PlanReason::InvalidLayout
    );
}
