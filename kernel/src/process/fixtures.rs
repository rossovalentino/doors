//! Preserved P05 protected-execution qualification fixtures.
use super::*;

pub fn verify(foundations: Foundations<'static>, archive: &door_initfs::Archive<'_>) {
    arch::disable_interrupts();
    let (frames, tables) = foundations.into_memory();
    let pids: [ProcessId; COUNT] =
        with_meta(|m| core::array::from_fn(|_| m.executive.create_process().unwrap()));
    let pair = with_meta(|m| m.executive.channel_pair(pids[0], pids[1])).unwrap();
    let inherited: [[Inherited; 8]; COUNT] = with_meta(|m| {
        core::array::from_fn(|i| {
            let mut handles = [Inherited::default(); 8];
            handles[0] = Inherited {
                handle: m
                    .executive
                    .process_handle(pids[i], pids[2], rights::WAIT | rights::QUERY)
                    .unwrap(),
                rights: rights::WAIT | rights::QUERY,
                kind: door_abi::HandleKind::Process as u64,
            };
            if i < 2 {
                let info = m.executive.inspect(pids[i], pair[i]).unwrap();
                handles[1] = Inherited {
                    handle: pair[i],
                    rights: info.rights,
                    kind: door_abi::HandleKind::Channel as u64,
                };
            }
            handles
        })
    });
    initialize_vm(frames, tables);
    let baseline = with_vm(|v| v.stats());
    let online = arch::online_mask() as u8;
    for i in 0..COUNT {
        let image = archive
            .file(if i == 4 {
                "bin/native-probe.elf"
            } else {
                "bin/native-runtime.elf"
            })
            .expect("native ELF in authenticated initfs");
        let plan = ImagePlan::parse(image).expect("native ELF plan");
        let startup = Startup {
            pid: pids[i].raw(),
            peer_pid: if i < 2 {
                pids[1 - i].raw()
            } else {
                pids[2].raw()
            },
            role: (i + 1) as u64,
            entry: plan.entry(),
            tls: 0x7100_2000,
            stack_top: 0x7001_1000,
            inherited: inherited[i],
        };
        let mut bytes = [0; 256];
        startup.encode(&mut bytes).expect("native startup");
        if i == 0 {
            let stages = with_vm(|v| unsafe {
                v.verify_startup_rollback(pids[i].slot(), image, &bytes, pids[i].raw())
            })
            .expect("startup allocation rollback");
            log(format_args!(
                "DOOR_P05 STARTUP_ROLLBACK PASS stages={stages} leaks=0\n"
            ));
        }
        let layout =
            with_vm(|v| unsafe { v.create_process(pids[i].slot(), image, &bytes, pids[i].raw()) })
                .expect("private process image");
        // This controlled fault is below the normal full-range validator. It
        // proves the architecture fixup restores the trusted CR3 and reports
        // only the copied prefix when a mapped page ends at a guard.
        with_vm(|_| {
            let mut tail = [0xa5; 16];
            assert_eq!(
                unsafe { arch::copy_from_user(layout.cr3, layout.user_stack_top - 8, &mut tail) },
                Err(arch::CopyError::Fault { copied: 8 })
            );
            assert_eq!(&tail[..8], &[0; 8]);
            assert_eq!(&tail[8..], &[0xa5; 8]);
            let current_cr3: u64;
            unsafe {
                asm!("mov {}, cr3",out(reg)current_cr3,options(nomem,nostack));
            }
            assert_eq!(current_cr3, arch::kernel_cr3());
        });
        let context = unsafe {
            arch::create_user_context(arch::UserContext {
                compat: false,
                entry: layout.entry,
                stack_pointer: layout.user_stack_top - 8,
                fs_base: layout.tls,
                user_gs_base: 0,
                cr3: layout.cr3,
                kernel_stack_top: layout.kernel_stack_top,
                arguments: arch::EntryArguments::Native {
                    startup: layout.startup,
                    bytes: 256,
                },
            })
        }
        .expect("ring3 context");
        let (task, exited) = run::with(|s| {
            (
                s.create(context as u64, 4, online).unwrap(),
                s.create_event(true, false).unwrap(),
            )
        });
        with_meta(|m| {
            m.records[task.slot()] = Some(Record {
                pid: pids[i],
                task,
                role: (i + 1) as u64,
                exited,
            })
        });
    }
    unsafe {
        arch::install_user_hooks(syscall, fault);
    }
    let records: [Record; COUNT] = with_meta(|m| {
        core::array::from_fn(|i| {
            *m.records
                .iter()
                .flatten()
                .find(|r| r.pid == pids[i])
                .unwrap()
        })
    });
    for r in records {
        run::with(|s| s.start(r.task)).unwrap();
    }
    arch::enable_timer_service(true);
    for cpu in 0..4 {
        if online & (1 << cpu) != 0 {
            arch::send_reschedule(cpu).unwrap();
        }
    }
    arch::enable_interrupts();
    let deadline = run::now() + 1500;
    while !run::with(|s| {
        records
            .iter()
            .all(|r| s.snapshot(r.task).unwrap().state == State::Terminated)
    }) {
        assert!(run::now() < deadline, "native process completion deadline");
        arch::wait_for_interrupt();
    }
    arch::disable_interrupts();
    for (r, expected) in records.iter().zip([11, 22, -14, 44, 55, -257]) {
        assert_eq!(
            with_meta(|m| m.executive.exit_code(r.pid)).unwrap(),
            Some(expected),
            "native fixture result"
        );
        run::with(|s| {
            assert_eq!(s.exit_result(r.task).unwrap(), Some(expected));
            s.reap(r.task).unwrap();
            s.close_object(r.exited).unwrap();
        });
        with_vm(|v| unsafe { v.destroy_process(r.pid.slot()) }).expect("retired process VM");
        for index in 0..MAX_VIEWS {
            with_meta(|m| {
                if let Some(view) = m.views[r.pid.slot()][index].take() {
                    m.executive.release_lease(view.lease.unwrap());
                }
            });
        }
        with_meta(|m| m.executive.reap(r.pid)).unwrap();
        with_meta(|m| m.records[r.task.slot()] = None);
        cleanup_sections();
    }
    let (sends, receives, maps) = with_meta(|m| {
        assert_eq!(m.executive.object_count(), 0);
        (m.ipc_sends, m.ipc_receives, m.section_maps)
    });
    assert!(
        sends >= 4 && receives >= 4 && maps >= 2,
        "native IPC path exercised"
    );
    let final_stats = with_vm(|v| v.stats());
    assert_eq!(final_stats.processes, 0);
    assert_eq!(final_stats.sections, 0);
    assert_eq!(final_stats.allocated_pages, baseline.allocated_pages);
    log(format_args!(
        "DOOR_P05 MEMORY PASS allocated_pages={} kernel_table_pages={} copy_faults=6\n",
        final_stats.allocated_pages, final_stats.kernel_table_pages
    ));
    arch::enable_timer_service(false);
    for cpu in 1..4 {
        if online & (1 << cpu) != 0 {
            arch::send_reschedule(cpu).unwrap();
        }
    }
    arch::stop_timer();
    log(format_args!(
        "DOOR_P05 NATIVE PASS processes=6 ring3=1 isolated=1\nDOOR_P05 IPC PASS authenticated=1 rights=1 shared=1\nDOOR_P05 IPC_LIFETIME PASS cancellation=1 deadline=1\nDOOR_P05 FAULTS PASS kernel_read=-14 invalid_return=-257 survivor=44\nDOOR_P05 ABI PASS independent_probe=55\nDOOR_P05 CLEANUP PASS retired=6 leaked=0\n"
    ));
}
