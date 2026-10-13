//! 已套用過的 migration 一改，使用者的資料庫就開不起來（sqlx 比對 checksum）。內容永遠不准改，要改就加新的。
use sha2::{Digest, Sha256};

const FROZEN: &[(&str, &str)] = &[
    ("0001_init.sql", "e6647ad44871ba1c2f666b5fd18c6063369bbe78d23f8872f258daa1870b4cff"),
    ("0002_approvals.sql", "6109de2aa55d3654de202b156988c80c239762ca3667a9332f0d98e841b888b9"),
    ("0003_approvals_issued.sql", "428f3fefad7ceb80ad855b09ad864e5a48d0f5a426e0519ff6300ea8e804752a"),
    ("0004_settings.sql", "0a3ec2420da190a25e904fe66d56b58cb81eff400ed753e0514d3a913bb86a54"),
    ("0005_deliveries.sql", "ced4a9ba68459aa485f5befb63c7f2214f0d996242f60f6632a0d0ead79ea7fc"),
    ("0006_ledger_outbox.sql", "9d749f28f9f496836fb92d39923ad64513116ead97a49f874cc96737e7943a80"),
    ("0007_money_outcomes.sql", "7ae2486f37cc655db531bdbd35124a0e09224aee317f4a99972db2e88ed1581f"),
];

#[test]
fn applied_migrations_are_never_modified() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    for (name, want) in FROZEN {
        let body = std::fs::read(dir.join(name)).expect(name);
        let got = hex::encode(Sha256::digest(&body));
        assert_eq!(&got, want, "{name} 已經發佈過，不能改內容（包括註解）；要改請新增一個 migration");
    }
}
