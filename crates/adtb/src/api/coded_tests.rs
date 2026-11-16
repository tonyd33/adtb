use crate::catalog::codec::coded;
use crate::catalog::codec::testing::{Sampled, read_back};
use crate::catalog::codec::{Coded, MigrationRow, RoleGrant, RoleRow, SettingRow};

use super::bundle::{Registered, Row};
use super::dump::{FeedMemberRow, FeedRow};
use super::server::{AuditRow, CredentialRow, PeerRow};
use super::verify::FindingRow;

/// Returns each way the samples of the codec of `c` fail to read back.
fn read_back_of(c: Coded) -> Vec<String> {
    fn of<T: Sampled>(c: Coded) -> Vec<String> {
        assert_eq!(c, coded::<T>());
        read_back::<T>()
    }
    match c {
        Coded::Roles => of::<RoleRow>(c),
        Coded::Grants => of::<RoleGrant>(c),
        Coded::Migrations => of::<MigrationRow>(c),
        Coded::Settings => of::<SettingRow>(c),
        Coded::Bundles => of::<Row>(c),
        Coded::Registered => of::<Registered>(c),
        Coded::Feeds => of::<FeedRow>(c),
        Coded::FeedMembers => of::<FeedMemberRow>(c),
        Coded::Credentials => of::<CredentialRow>(c),
        Coded::PeerMappings => of::<PeerRow>(c),
        Coded::Audit => of::<AuditRow>(c),
        Coded::VerifyFindings => of::<FindingRow>(c),
    }
}

#[test]
fn every_codec_writes_and_reads_its_samples_under_the_shipped_and_both_reversed_catalogs() {
    let failed: Vec<String> = Coded::ALL.iter().copied().flat_map(read_back_of).collect();
    assert_eq!(Vec::<String>::new(), failed);
}
