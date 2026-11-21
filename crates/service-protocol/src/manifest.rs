//! The initial native service graph. Privileges remain kernel catalog policy;
//! this declaration supplies the user supervisor's readiness/restart contract.
use crate::{
    opcode,
    supervisor::{Dependency, Manifest, RestartPolicy},
};
pub use door_abi::platform::manifest::{CONSOLE, INIT, SESSION, STORAGE, WORKLOAD};
use door_abi::platform::{MANIFEST_POLICIES, ManifestPolicy};
pub const MAX_CLIENT_CONNECTIONS: usize = 16;
pub const MAX_OPEN_FILES: usize = 256;
pub const MAX_OPEN_KEYS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Readiness {
    AuthenticatedPort,
    RecoveredCatalogAndPort,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistentState {
    None,
    JournalV1CatalogV1SettingsV2,
    KernelSessionEpochs,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Shutdown {
    ReverseDependencyDrain,
    CloseHandlesAndCommit,
    CloseSessionAuthority,
    ReleaseTerminalInput,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Endpoint {
    pub family: &'static str,
    pub major: u16,
    pub minor: u16,
    pub operations: &'static [u32],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceContract {
    pub id: u64,
    pub policy: ManifestPolicy,
    pub endpoints: &'static [Endpoint],
    pub max_connections: usize,
    pub file_handles: usize,
    pub key_handles: usize,
    pub readiness: Readiness,
    pub persistent: PersistentState,
    pub shutdown: Shutdown,
}
const REGISTRY_ENDPOINTS: &[Endpoint] = &[Endpoint {
    family: "door.service",
    major: 1,
    minor: 0,
    operations: &[
        opcode::RESOLVE,
        crate::application::EXECUTE,
        crate::application::POWER,
        crate::application::CANCEL_CHILDREN,
        crate::application::EXECUTE_DETACHED,
        crate::application::ARGUMENTS_STAGE,
        crate::application::ARGUMENTS_FETCH,
    ],
}];
const STORAGE_ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        family: "door.storage",
        major: 1,
        minor: 0,
        operations: &[
            opcode::OPEN,
            opcode::OPEN_WINDOWS,
            opcode::READ,
            opcode::WRITE,
            opcode::FLUSH,
            opcode::CLOSE,
            opcode::METADATA,
            opcode::FILE_INFO,
            opcode::LIST,
            opcode::MKDIR,
            opcode::RENAME,
            opcode::RENAME_CHILD_CHECKED,
            opcode::REPLACE_BEGIN,
            opcode::REPLACE_CHUNK,
            opcode::REPLACE_COMMIT,
            opcode::REPLACE_ABORT,
            opcode::REMOVE,
            opcode::REMOVE_CHILD_CHECKED,
            opcode::TRUNCATE,
            opcode::OPEN_FOR,
            opcode::METADATA_FOR,
            opcode::DISK_INFO,
            opcode::VOLUME_USAGE,
            opcode::SECURITY,
            opcode::SET_SECURITY,
            opcode::FILE_SECURITY,
            opcode::SET_FILE_SECURITY,
        ],
    },
    Endpoint {
        family: "door.config",
        major: 1,
        minor: 0,
        operations: &[
            opcode::GET_SETTING,
            opcode::SET_SETTING,
            opcode::GET_SETTING_FOR,
            opcode::SET_SETTING_FOR,
        ],
    },
    Endpoint {
        family: "door.registry",
        major: 1,
        minor: 0,
        operations: &[
            opcode::REGISTRY_OPEN,
            opcode::REGISTRY_QUERY,
            opcode::REGISTRY_SET,
            opcode::REGISTRY_CLOSE,
            opcode::REGISTRY_VERSION,
            opcode::REGISTRY_ENUM_KEY,
            opcode::REGISTRY_ENUM_VALUE,
            opcode::REGISTRY_SET_CHECKED,
            opcode::REGISTRY_DELETE_VALUE,
            opcode::REGISTRY_DELETE_KEY,
            opcode::REGISTRY_STAGE_VALUE,
        ],
    },
];
const SESSION_ENDPOINTS: &[Endpoint] = &[Endpoint {
    family: "door.session",
    major: 1,
    minor: 0,
    operations: &[opcode::LOGON, opcode::LOGOFF],
}];
const CONSOLE_ENDPOINTS: &[Endpoint] = &[Endpoint {
    family: "door.console",
    major: 1,
    minor: 0,
    operations: &[
        opcode::CONSOLE_WRITE,
        opcode::CONSOLE_READ,
        opcode::CONSOLE_CONTROL,
        opcode::CONSOLE_INTERRUPT,
        opcode::CONSOLE_MIRROR,
        opcode::CONSOLE_INPUT,
        opcode::CONSOLE_STATE,
        opcode::CONSOLE_READ_INPUT,
    ],
}];
pub const CONTRACTS: [ServiceContract; 4] = [
    ServiceContract {
        id: INIT,
        policy: MANIFEST_POLICIES[0],
        endpoints: REGISTRY_ENDPOINTS,
        max_connections: MAX_CLIENT_CONNECTIONS,
        file_handles: 0,
        key_handles: 0,
        readiness: Readiness::AuthenticatedPort,
        persistent: PersistentState::None,
        shutdown: Shutdown::ReverseDependencyDrain,
    },
    ServiceContract {
        id: STORAGE,
        policy: MANIFEST_POLICIES[1],
        endpoints: STORAGE_ENDPOINTS,
        max_connections: MAX_CLIENT_CONNECTIONS,
        file_handles: MAX_OPEN_FILES,
        key_handles: MAX_OPEN_KEYS,
        readiness: Readiness::RecoveredCatalogAndPort,
        persistent: PersistentState::JournalV1CatalogV1SettingsV2,
        shutdown: Shutdown::CloseHandlesAndCommit,
    },
    ServiceContract {
        id: SESSION,
        policy: MANIFEST_POLICIES[2],
        endpoints: SESSION_ENDPOINTS,
        max_connections: MAX_CLIENT_CONNECTIONS,
        file_handles: 0,
        key_handles: 0,
        readiness: Readiness::AuthenticatedPort,
        persistent: PersistentState::KernelSessionEpochs,
        shutdown: Shutdown::CloseSessionAuthority,
    },
    ServiceContract {
        id: CONSOLE,
        policy: MANIFEST_POLICIES[3],
        endpoints: CONSOLE_ENDPOINTS,
        max_connections: MAX_CLIENT_CONNECTIONS,
        file_handles: 0,
        key_handles: 0,
        readiness: Readiness::AuthenticatedPort,
        persistent: PersistentState::None,
        shutdown: Shutdown::ReleaseTerminalInput,
    },
];
pub const PORT_RIGHTS: u64 =
    door_abi::rights::QUERY | door_abi::rights::WRITE | door_abi::rights::TRANSFER;
pub const PUBLISH_RIGHTS: u64 = PORT_RIGHTS | door_abi::rights::DUPLICATE;
pub const CONTROL_RIGHTS: u64 =
    door_abi::rights::QUERY | door_abi::rights::READ | door_abi::rights::WRITE;
const RESTART: RestartPolicy = RestartPolicy {
    limit: 3,
    base: 10,
    max: 40,
    stable: 1000,
};
static NEED_STORAGE: [Dependency; 1] = [Dependency {
    service: "door-storage",
    optional: false,
}];
static NEED_SESSION: [Dependency; 1] = [Dependency {
    service: "door-session",
    optional: false,
}];
pub static SERVICES: [Manifest; 3] = [
    Manifest {
        name: "door-storage",
        image: "bin/door-storage.elf",
        account: MANIFEST_POLICIES[1].uid,
        dependencies: &[],
        ready_deadline: 500,
        stop_deadline: 100,
        restart: RESTART,
        essential: true,
        autostart: true,
    },
    Manifest {
        name: "door-session",
        image: "bin/door-session.elf",
        account: MANIFEST_POLICIES[2].uid,
        dependencies: &NEED_STORAGE,
        ready_deadline: 200,
        stop_deadline: 100,
        restart: RESTART,
        essential: true,
        autostart: true,
    },
    Manifest {
        name: "door-console",
        image: "bin/door-console.elf",
        account: MANIFEST_POLICIES[3].uid,
        dependencies: &NEED_SESSION,
        ready_deadline: 200,
        stop_deadline: 100,
        restart: RESTART,
        essential: false,
        autostart: true,
    },
];
pub const IDS: [u64; 3] = [STORAGE, SESSION, CONSOLE];

#[cfg(test)]
mod tests {
    #[test]
    fn contracts_reuse_kernel_policy_and_publish_unique_versioned_operations() {
        for contract in super::CONTRACTS {
            assert_eq!(
                Some(contract.policy),
                door_abi::platform::manifest_policy(contract.id)
            );
            assert_eq!(contract.max_connections, super::MAX_CLIENT_CONNECTIONS);
            assert_eq!(contract.policy.max_threads, 1);
            // Session has a heap for the Argon2id working memory of the P14
            // credential store (ADR 0020), Console for its pseudoconsoles.
            let heap = match contract.id {
                door_abi::platform::manifest::SESSION => 8 * door_abi::platform::HEAP_BYTES,
                door_abi::platform::manifest::CONSOLE => door_abi::platform::CONSOLE_HEAP_BYTES,
                _ => 0,
            };
            assert_eq!(contract.policy.max_heap_bytes, heap);
            for (i, endpoint) in contract.endpoints.iter().enumerate() {
                assert!(!endpoint.family.is_empty());
                assert_eq!((endpoint.major, endpoint.minor), (1, 0));
                for (j, operation) in endpoint.operations.iter().enumerate() {
                    assert!(!endpoint.operations[..j].contains(operation));
                    assert!(
                        contract.endpoints[..i]
                            .iter()
                            .all(|e| !e.operations.contains(operation))
                    );
                }
            }
        }
        assert!(door_abi::platform::manifest_policy(0).is_none());
        assert!(door_abi::platform::manifest_policy(u64::MAX).is_none());
        let workload = door_abi::platform::manifest_policy(super::WORKLOAD).unwrap();
        assert_eq!(
            (workload.uid, workload.caps, workload.max_threads),
            (0, 0, 2)
        );
        assert_eq!(workload.max_heap_bytes, door_abi::platform::HEAP_BYTES);
    }
    #[test]
    fn shipped_graph_is_acyclic_and_storage_is_first() {
        let mut policy = crate::supervisor::Supervisor::new(&super::SERVICES).unwrap();
        let actions = policy.step(0);
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions.iter().next(),
            Some(crate::supervisor::Action::Launch { service: 0, .. })
        ));
    }
}
