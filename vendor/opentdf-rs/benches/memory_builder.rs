use base64::Engine;
use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use opentdf::{
    AttributeIdentifier, AttributePolicy, AttributeValue, Policy, PolicyBody, TdfArchive,
    TdfArchiveMemoryBuilder, TdfEncryption, TdfManifest, manifest::TdfManifestExt,
};
use std::io::Cursor;

// Helper function to create test data of specific size
fn create_test_data(size_kb: usize) -> Vec<u8> {
    vec![0u8; size_kb * 1024]
}

// Helper function to create a test policy
fn create_test_policy() -> Policy {
    let attr_id =
        AttributeIdentifier::new("https://example.com".to_string(), "clearance".to_string());
    let attr_value = AttributeValue::String("secret".to_string());
    let attr_policy = AttributePolicy::condition(attr_id, opentdf::Operator::Equals, attr_value);

    Policy {
        uuid: uuid::Uuid::new_v4().to_string(),
        valid_from: None,
        valid_to: None,
        body: PolicyBody {
            attributes: vec![attr_policy],
            dissem: Vec::new(),
        },
    }
}

// Benchmark: TDF Encryption only
fn bench_encryption(c: &mut Criterion) {
    let mut group = c.benchmark_group("encryption");

    for size_kb in [1, 10, 100, 1000].iter() {
        let data = create_test_data(*size_kb);
        group.throughput(Throughput::Bytes((size_kb * 1024) as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size_kb), size_kb, |b, _| {
            b.iter(|| {
                let tdf_encryption = TdfEncryption::new().expect("Failed to create encryption");
                let _encrypted = tdf_encryption
                    .encrypt(black_box(&data))
                    .expect("Failed to encrypt");
            });
        });
    }

    group.finish();
}

// Benchmark: In-memory archive building
fn bench_memory_archive_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_archive_building");

    for size_kb in [1, 10, 100, 1000].iter() {
        let data = create_test_data(*size_kb);
        let tdf_encryption = TdfEncryption::new().expect("Failed to create encryption");
        let encrypted_payload = tdf_encryption
            .encrypt(&data)
            .expect("Failed to encrypt data");

        // Decode ciphertext from base64
        let ciphertext_bytes = base64::engine::general_purpose::STANDARD
            .decode(&encrypted_payload.ciphertext)
            .expect("Failed to decode ciphertext");

        let manifest = TdfManifest::new(
            "0.payload".to_string(),
            "https://kas.example.com".to_string(),
        );

        group.throughput(Throughput::Bytes((size_kb * 1024) as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size_kb), size_kb, |b, _| {
            b.iter(|| {
                let mut builder = TdfArchiveMemoryBuilder::new();
                builder
                    .add_entry(black_box(&manifest), black_box(&ciphertext_bytes), 0)
                    .expect("Failed to add entry");
                let _result = builder.finish().expect("Failed to finish archive");
            });
        });
    }

    group.finish();
}

// Benchmark: Complete TDF creation (encryption + archive building)
fn bench_complete_tdf_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("complete_tdf_creation");

    for size_kb in [1, 10, 100, 1000].iter() {
        let data = create_test_data(*size_kb);
        let policy = create_test_policy();

        group.throughput(Throughput::Bytes((size_kb * 1024) as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size_kb), size_kb, |b, _| {
            b.iter(|| {
                // Create encryption instance
                let tdf_encryption = TdfEncryption::new().expect("Failed to create encryption");

                // Encrypt data
                let encrypted_payload = tdf_encryption
                    .encrypt(black_box(&data))
                    .expect("Failed to encrypt");

                // Decode ciphertext
                let ciphertext_bytes = base64::engine::general_purpose::STANDARD
                    .decode(&encrypted_payload.ciphertext)
                    .expect("Failed to decode ciphertext");

                // Create manifest
                let mut manifest = TdfManifest::new(
                    "0.payload".to_string(),
                    "https://kas.example.com".to_string(),
                );
                manifest
                    .set_policy(black_box(&policy))
                    .expect("Failed to set policy");
                manifest.encryption_information.method.iv = encrypted_payload.iv.clone();

                // Update key access
                if let Some(key_access) = manifest.encryption_information.key_access.first_mut() {
                    key_access.wrapped_key = encrypted_payload.encrypted_key.clone();
                    key_access.policy_binding = opentdf::manifest::PolicyBinding {
                        alg: "HS256".to_string(),
                        hash: encrypted_payload.policy_key_hash.clone(),
                    };
                }

                // Build archive
                let mut builder = TdfArchiveMemoryBuilder::new();
                builder
                    .add_entry(&manifest, &ciphertext_bytes, 0)
                    .expect("Failed to add entry");
                let _tdf_bytes = builder.finish().expect("Failed to finish archive");
            });
        });
    }

    group.finish();
}

// Benchmark: TDF reading from memory
fn bench_tdf_reading(c: &mut Criterion) {
    let mut group = c.benchmark_group("tdf_reading");

    for size_kb in [1, 10, 100, 1000].iter() {
        let data = create_test_data(*size_kb);
        let policy = create_test_policy();

        // Create a TDF archive in memory
        let tdf_encryption = TdfEncryption::new().expect("Failed to create encryption");
        let encrypted_payload = tdf_encryption.encrypt(&data).expect("Failed to encrypt");
        let ciphertext_bytes = base64::engine::general_purpose::STANDARD
            .decode(&encrypted_payload.ciphertext)
            .expect("Failed to decode ciphertext");

        let mut manifest = TdfManifest::new(
            "0.payload".to_string(),
            "https://kas.example.com".to_string(),
        );
        manifest.set_policy(&policy).expect("Failed to set policy");
        manifest.encryption_information.method.iv = encrypted_payload.iv.clone();

        if let Some(key_access) = manifest.encryption_information.key_access.first_mut() {
            key_access.wrapped_key = encrypted_payload.encrypted_key.clone();
            key_access.policy_binding = opentdf::manifest::PolicyBinding {
                alg: "HS256".to_string(),
                hash: encrypted_payload.policy_key_hash.clone(),
            };
        }

        let mut builder = TdfArchiveMemoryBuilder::new();
        builder
            .add_entry(&manifest, &ciphertext_bytes, 0)
            .expect("Failed to add entry");
        let tdf_bytes = builder.finish().expect("Failed to finish archive");

        group.throughput(Throughput::Bytes((size_kb * 1024) as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size_kb), size_kb, |b, _| {
            b.iter(|| {
                let cursor = Cursor::new(black_box(&tdf_bytes));
                let mut archive = TdfArchive::new(cursor).expect("Failed to read archive");
                let entry = archive.get_entry(0).expect("Failed to get entry");
                let _manifest = &entry.manifest;
            });
        });
    }

    group.finish();
}

// Benchmark: Policy validation
fn bench_policy_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("policy_operations");

    group.bench_function("create_simple_policy", |b| {
        b.iter(|| {
            let attr_id = AttributeIdentifier::new(
                black_box("https://example.com".to_string()),
                black_box("clearance".to_string()),
            );
            let attr_value = AttributeValue::String(black_box("secret".to_string()));
            let _policy =
                AttributePolicy::condition(attr_id, opentdf::Operator::Equals, attr_value);
        });
    });

    group.bench_function("create_complex_policy", |b| {
        b.iter(|| {
            let attr_id1 = AttributeIdentifier::new(
                black_box("https://example.com".to_string()),
                black_box("clearance".to_string()),
            );
            let attr_id2 = AttributeIdentifier::new(
                black_box("https://example.com".to_string()),
                black_box("department".to_string()),
            );

            let condition1 = AttributePolicy::condition(
                attr_id1,
                opentdf::Operator::Equals,
                AttributeValue::String(black_box("secret".to_string())),
            );
            let condition2 = AttributePolicy::condition(
                attr_id2,
                opentdf::Operator::Equals,
                AttributeValue::String(black_box("engineering".to_string())),
            );

            let _policy = AttributePolicy::and(vec![condition1, condition2]);
        });
    });

    group.bench_function("serialize_policy", |b| {
        let policy = create_test_policy();
        b.iter(|| {
            let _json = serde_json::to_string(black_box(&policy)).expect("Failed to serialize");
        });
    });

    group.bench_function("deserialize_policy", |b| {
        let policy = create_test_policy();
        let json = serde_json::to_string(&policy).expect("Failed to serialize");
        b.iter(|| {
            let _policy: Policy =
                serde_json::from_str(black_box(&json)).expect("Failed to deserialize");
        });
    });

    group.finish();
}

// Benchmark: Manifest operations
fn bench_manifest_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("manifest_operations");

    group.bench_function("create_manifest", |b| {
        b.iter(|| {
            let _manifest = TdfManifest::new(
                black_box("0.payload".to_string()),
                black_box("https://kas.example.com".to_string()),
            );
        });
    });

    group.bench_function("serialize_manifest", |b| {
        let manifest = TdfManifest::new(
            "0.payload".to_string(),
            "https://kas.example.com".to_string(),
        );
        b.iter(|| {
            let _json = manifest.to_json().expect("Failed to serialize");
        });
    });

    group.bench_function("deserialize_manifest", |b| {
        let manifest = TdfManifest::new(
            "0.payload".to_string(),
            "https://kas.example.com".to_string(),
        );
        let json = manifest.to_json().expect("Failed to serialize");
        b.iter(|| {
            let _manifest =
                TdfManifest::from_json(black_box(&json)).expect("Failed to deserialize");
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_encryption,
    bench_memory_archive_building,
    bench_complete_tdf_creation,
    bench_tdf_reading,
    bench_policy_operations,
    bench_manifest_operations
);
criterion_main!(benches);
