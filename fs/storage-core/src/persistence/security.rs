use super::{Error, Result};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Access(pub u16);
impl Access {
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const DELETE: Self = Self(4);
    pub const TRAVERSE: Self = Self(8);
    pub const CREATE: Self = Self(16);
    pub const CHANGE_ACL: Self = Self(32);
    pub const ALL: Self = Self(63);
    pub const SHARE_ALL: Self = Self(7);
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub(crate) fn valid(self) -> bool {
        self.0 & !63 == 0
    }
}
/// Immutable captured effective identity, never a path or displayed username.
#[derive(Clone, Copy, Debug)]
pub struct Subject {
    pub(crate) id: u32,
    groups: u32,
    integrity: u8,
}
impl Subject {
    /// # Safety
    /// The caller is the trusted authentication/executive boundary and has
    /// authenticated these claims. Never populate this from user request bytes.
    pub unsafe fn authenticated(id: u32, groups: u32, integrity: u8) -> Result<Self> {
        if id == 0 || integrity > 2 {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            id,
            groups,
            integrity,
        })
    }
    /// The system identity of a host tool provisioning a volume image
    /// offline (`cargo xtask`), as the storage manifest's identity
    /// provisions a new volume; no guest service enables this.
    #[cfg(feature = "offline-provisioning")]
    pub fn offline_system() -> Self {
        Self {
            id: 1,
            groups: 0,
            integrity: 2,
        }
    }
    pub fn identity(self) -> u32 {
        self.id
    }
    pub(crate) fn system(self) -> bool {
        self.id == 1 && self.integrity == 2
    }
    pub(crate) fn integrity(self) -> u8 {
        self.integrity
    }
    /// BUILTIN\Administrators enabled (an elevated process: User Account
    /// Control approved it), or the system itself.
    pub fn administrator(self) -> bool {
        self.system() || self.groups & (1 << ADMINISTRATORS) != 0
    }
    /// The owner of the files and folders it creates: Administrators for an
    /// elevated process (as Windows' elevated token's default owner), else
    /// itself.
    pub fn default_owner(self) -> u32 {
        if !self.system() && self.administrator() {
            ADMINISTRATORS_OWNER
        } else {
            self.id
        }
    }
    /// Whether it holds the owner's implicit rights on an object `owner`
    /// owns (itself, or Administrators while elevated), or may make
    /// `owner` the owner.
    pub fn owns(self, owner: u32) -> bool {
        if owner == ADMINISTRATORS_OWNER {
            self.administrator()
        } else {
            owner == self.id
        }
    }
}
pub use door_service_protocol::files::ADMINISTRATORS_OWNER;
/// The group bit of BUILTIN\Administrators in an authenticated subject
/// (`door_abi::platform::group::ADMINISTRATORS` is `1 << 3`), for
/// `Principal::Group` entries.
pub const ADMINISTRATORS: u8 = 3;
/// The account is an administrator (`group::ADMINISTRATOR_ACCOUNT`, 1 << 4):
/// unelevated, its token holds Administrators for deny only, so an entry
/// denying Administrators applies to it too.
pub const ADMINISTRATOR_ACCOUNT: u8 = 4;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Principal {
    Everyone,
    User(u32),
    Group(u8),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ace {
    pub principal: Principal,
    pub deny: bool,
    pub access: Access,
    pub inherit: bool,
}
impl Ace {
    pub const EMPTY: Self = Self {
        principal: Principal::Everyone,
        deny: false,
        access: Access(0),
        inherit: false,
    };
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Acl {
    pub owner: u32,
    pub integrity: u8,
    pub(crate) count: u8,
    pub(crate) entries: [Ace; 4],
}
impl Acl {
    pub const fn empty(owner: u32, integrity: u8) -> Self {
        Self {
            owner,
            integrity,
            count: 0,
            entries: [Ace::EMPTY; 4],
        }
    }
    pub fn push(&mut self, ace: Ace) -> Result<()> {
        if self.count == 4 {
            return Err(Error::NoSpace);
        }
        if !ace.access.valid()
            || matches!(ace.principal, Principal::User(0) | Principal::Group(32..))
        {
            return Err(Error::InvalidParameter);
        }
        self.entries[self.count as usize] = ace;
        self.count += 1;
        Ok(())
    }
    /// The entries in order (the first matching deny refuses).
    pub fn entries(&self) -> &[Ace] {
        &self.entries[..usize::from(self.count.min(4))]
    }
    pub fn owner_full(owner: u32) -> Self {
        let mut acl = Self::empty(owner, 0);
        acl.entries[0] = Ace {
            principal: Principal::User(owner),
            deny: false,
            access: Access::ALL,
            inherit: true,
        };
        acl.count = 1;
        acl
    }
    pub(crate) fn valid(&self) -> bool {
        self.owner != 0
            && self.integrity <= 2
            && self.count <= 4
            && self.entries[..self.count as usize].iter().all(|a| {
                a.access.valid()
                    && !matches!(a.principal, Principal::User(0) | Principal::Group(32..))
            })
    }
    pub fn check(&self, subject: Subject, requested: Access) -> Result<()> {
        if !self.valid() || !requested.valid() {
            return Err(Error::InvalidParameter);
        }
        if subject.integrity < self.integrity && requested.0 & (2 | 4 | 16 | 32) != 0 {
            return Err(Error::AccessDenied);
        }
        let mut remaining = requested.0;
        for ace in &self.entries[..self.count as usize] {
            if remaining == 0 {
                return Ok(());
            }
            let applies = match ace.principal {
                Principal::Everyone => true,
                Principal::User(id) => id == subject.id,
                Principal::Group(g) => {
                    subject.groups & (1u32 << g) != 0
                        || (ace.deny
                            && g == ADMINISTRATORS
                            && subject.groups & (1u32 << ADMINISTRATOR_ACCOUNT) != 0)
                }
            };
            if applies {
                if ace.deny && ace.access.0 & remaining != 0 {
                    return Err(Error::AccessDenied);
                }
                if !ace.deny {
                    remaining &= !ace.access.0;
                }
            }
        }
        if remaining != 0 {
            Err(Error::AccessDenied)
        } else {
            Ok(())
        }
    }
    pub(crate) fn inherited(self, owner: u32) -> Self {
        let mut next = Self::empty(owner, self.integrity);
        for &ace in &self.entries[..self.count as usize] {
            if ace.inherit {
                next.entries[next.count as usize] = ace;
                next.count += 1;
            }
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn denying_administrators_also_stops_an_unelevated_administrator() {
        let mut acl = Acl::empty(1, 0);
        acl.push(Ace {
            principal: Principal::Group(ADMINISTRATORS),
            deny: true,
            access: Access::WRITE,
            inherit: false,
        })
        .unwrap();
        acl.push(Ace {
            principal: Principal::Everyone,
            deny: false,
            access: Access::ALL,
            inherit: false,
        })
        .unwrap();
        let subject = |groups| unsafe { Subject::authenticated(1001, groups, 1).unwrap() };
        // An administrator's account, unelevated (deny-only), and elevated.
        assert!(
            acl.check(subject(1 << ADMINISTRATOR_ACCOUNT), Access::WRITE)
                .is_err()
        );
        assert!(
            acl.check(subject(1 << ADMINISTRATORS), Access::WRITE)
                .is_err()
        );
        assert!(
            acl.check(subject(1 << ADMINISTRATOR_ACCOUNT), Access::READ)
                .is_ok()
        );
        // A standard user.
        assert!(acl.check(subject(1 << 1), Access::WRITE).is_ok());
        // Allow entries for Administrators still need the enabled group.
        let mut allow = Acl::empty(1, 0);
        allow
            .push(Ace {
                principal: Principal::Group(ADMINISTRATORS),
                deny: false,
                access: Access::WRITE,
                inherit: false,
            })
            .unwrap();
        assert!(
            allow
                .check(subject(1 << ADMINISTRATOR_ACCOUNT), Access::WRITE)
                .is_err()
        );
        assert!(
            allow
                .check(subject(1 << ADMINISTRATORS), Access::WRITE)
                .is_ok()
        );
    }
}
