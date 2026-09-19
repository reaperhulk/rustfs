// Copyright 2024 RustFS Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use rustfs_filemeta::{
    FileInfo, FileInfoOpts, FileMeta, MetaCacheEntry, MetaObject, ObjectPartInfo, get_file_info, test_data::*,
};
use std::hint::black_box;

fn bench_create_real_xlmeta(c: &mut Criterion) {
    c.bench_function("create_real_xlmeta", |b| b.iter(|| black_box(create_real_xlmeta().unwrap())));
}

fn bench_create_complex_xlmeta(c: &mut Criterion) {
    c.bench_function("create_complex_xlmeta", |b| b.iter(|| black_box(create_complex_xlmeta().unwrap())));
}

fn bench_parse_real_xlmeta(c: &mut Criterion) {
    let data = create_real_xlmeta().unwrap();

    c.bench_function("parse_real_xlmeta", |b| b.iter(|| black_box(FileMeta::load(&data).unwrap())));
}

fn bench_parse_complex_xlmeta(c: &mut Criterion) {
    let data = create_complex_xlmeta().unwrap();

    c.bench_function("parse_complex_xlmeta", |b| b.iter(|| black_box(FileMeta::load(&data).unwrap())));
}

fn bench_serialize_real_xlmeta(c: &mut Criterion) {
    let data = create_real_xlmeta().unwrap();
    let fm = FileMeta::load(&data).unwrap();

    c.bench_function("serialize_real_xlmeta", |b| b.iter(|| black_box(fm.marshal_msg().unwrap())));
}

fn bench_serialize_complex_xlmeta(c: &mut Criterion) {
    let data = create_complex_xlmeta().unwrap();
    let fm = FileMeta::load(&data).unwrap();

    c.bench_function("serialize_complex_xlmeta", |b| b.iter(|| black_box(fm.marshal_msg().unwrap())));
}

fn bench_round_trip_real_xlmeta(c: &mut Criterion) {
    let original_data = create_real_xlmeta().unwrap();

    c.bench_function("round_trip_real_xlmeta", |b| {
        b.iter(|| {
            let fm = FileMeta::load(&original_data).unwrap();
            let serialized = fm.marshal_msg().unwrap();
            black_box(FileMeta::load(&serialized).unwrap())
        })
    });
}

fn bench_round_trip_complex_xlmeta(c: &mut Criterion) {
    let original_data = create_complex_xlmeta().unwrap();

    c.bench_function("round_trip_complex_xlmeta", |b| {
        b.iter(|| {
            let fm = FileMeta::load(&original_data).unwrap();
            let serialized = fm.marshal_msg().unwrap();
            black_box(FileMeta::load(&serialized).unwrap())
        })
    });
}

fn bench_version_stats(c: &mut Criterion) {
    let data = create_complex_xlmeta().unwrap();
    let fm = FileMeta::load(&data).unwrap();

    c.bench_function("version_stats", |b| b.iter(|| black_box(fm.get_version_stats())));
}

fn bench_into_fileinfo_realistic(c: &mut Criterion) {
    let data = create_realistic_object_xlmeta().unwrap();
    let fm = FileMeta::load(&data).unwrap();

    c.bench_function("into_fileinfo_realistic", |b| {
        b.iter(|| {
            black_box(
                fm.into_fileinfo(black_box("bucket"), black_box("object"), black_box(""), false, false, true)
                    .unwrap(),
            )
        })
    });
}

fn bench_validate_integrity(c: &mut Criterion) {
    let data = create_real_xlmeta().unwrap();
    let fm = FileMeta::load(&data).unwrap();

    c.bench_function("validate_integrity", |b| {
        b.iter(|| {
            fm.validate_integrity().unwrap();
            black_box(())
        })
    });
}

// Simulates the per-object CPU cost of a LIST page: each entry carries raw xl.meta bytes
// (cached = None), so to_fileinfo runs the full FileMeta::load + into_fileinfo path that a
// listing performs for every returned object. Reports objects/sec via Throughput::Elements.
fn bench_list_page_to_fileinfo(c: &mut Criterion) {
    const N: u64 = 10_000;
    let data = create_realistic_object_xlmeta().unwrap();
    let entries: Vec<MetaCacheEntry> = (0..N)
        .map(|i| MetaCacheEntry {
            name: format!("prefix/object-{i:08}"),
            metadata: data.clone(),
            cached: None,
            reusable: false,
        })
        .collect();

    let mut group = c.benchmark_group("list_page_to_fileinfo");
    group.throughput(Throughput::Elements(N));
    group.bench_function("realistic_10k", |b| {
        b.iter(|| {
            for entry in &entries {
                black_box(entry.to_fileinfo(black_box("bucket")).unwrap());
            }
        })
    });
    group.finish();
}

// Per-disk metadata work shared by ordinary HEAD/GET and the PUT commit path.
// The object is an uncompressed, single-part video with no optional features.
fn bench_object_metadata(c: &mut Criterion) {
    let mut fi = FileInfo::new("video/S06E21/clip.mp4", 4, 2);
    fi.volume = "bucket".to_owned();
    fi.name = "video/S06E21/clip.mp4".to_owned();
    fi.size = 4 * 1024 * 1024;
    fi.erasure.index = 1;
    fi.data_dir = Some(uuid::Uuid::from_u128(1));
    fi.mod_time = Some(time::OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("valid fixture timestamp"));
    fi.metadata = [
        ("etag".to_owned(), "f5e89f73c9242db8f5e984bce5ab3926".to_owned()),
        ("content-type".to_owned(), "video/mp4".to_owned()),
    ]
    .into_iter()
    .collect();
    fi.parts = vec![ObjectPartInfo {
        number: 1,
        size: 4 * 1024 * 1024,
        actual_size: fi.size,
        ..Default::default()
    }];
    let mut meta = FileMeta::new();
    meta.add_version(fi.clone()).expect("encode fixture version");
    let encoded = meta.marshal_msg().expect("encode fixture metadata");
    let mut group = c.benchmark_group("object_metadata/plain_video");
    group.throughput(Throughput::Elements(1));
    let object = MetaObject::from(fi.clone());
    group.bench_function("signature", |b| b.iter(|| black_box(black_box(&object).get_signature())));
    group.bench_function("decode_validate", |b| {
        b.iter(|| {
            let decoded = get_file_info(
                black_box(&encoded),
                "bucket",
                "video/S06E21/clip.mp4",
                "",
                FileInfoOpts {
                    data: true,
                    include_free_versions: false,
                    include_part_checksums: false,
                },
            )
            .expect("decode object metadata");
            decoded.validate_for_metadata_read().expect("validate object metadata");
            black_box(decoded);
        })
    });
    group.bench_function("create_version", |b| {
        b.iter(|| {
            let mut created = FileMeta::new();
            created.add_version(fi.clone()).expect("create object metadata");
            black_box(created.marshal_msg().expect("encode new object metadata"));
        })
    });
    group.bench_function("read_modify_write", |b| {
        b.iter(|| {
            let mut loaded = FileMeta::load(black_box(&encoded)).expect("load object metadata");
            loaded.add_version(fi.clone()).expect("update object metadata");
            black_box(loaded.marshal_msg().expect("encode updated object metadata"));
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_create_real_xlmeta,
    bench_create_complex_xlmeta,
    bench_parse_real_xlmeta,
    bench_parse_complex_xlmeta,
    bench_serialize_real_xlmeta,
    bench_serialize_complex_xlmeta,
    bench_round_trip_real_xlmeta,
    bench_round_trip_complex_xlmeta,
    bench_version_stats,
    bench_validate_integrity,
    bench_into_fileinfo_realistic,
    bench_list_page_to_fileinfo,
    bench_object_metadata
);

criterion_main!(benches);
