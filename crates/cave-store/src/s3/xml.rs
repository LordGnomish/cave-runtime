//! S3 XML response serialization.
//!
//! S3 uses XML for most responses. This module builds XML strings directly
//! rather than relying on serde reflection, giving full control over element names.

use chrono::{DateTime, Utc};

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn s3_date(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S.000Z").to_string()
}

// ── Error ─────────────────────────────────────────────────────────────────────

pub fn error_response(code: &str, message: &str, resource: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Error>
  <Code>{code}</Code>
  <Message>{message}</Message>
  <Resource>{resource}</Resource>
  <RequestId>cave-store-{ts}</RequestId>
</Error>"#,
        code = xml_escape(code),
        message = xml_escape(message),
        resource = xml_escape(resource),
        ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
    )
}

// ── ListBuckets ───────────────────────────────────────────────────────────────

pub struct BucketListItem {
    pub name: String,
    pub creation_date: DateTime<Utc>,
}

pub fn list_buckets(owner: &str, buckets: &[BucketListItem]) -> String {
    let mut bucket_xml = String::new();
    for b in buckets {
        bucket_xml.push_str(&format!(
            "    <Bucket><Name>{}</Name><CreationDate>{}</CreationDate></Bucket>\n",
            xml_escape(&b.name),
            s3_date(&b.creation_date),
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ListAllMyBucketsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Owner><ID>{owner}</ID><DisplayName>{owner}</DisplayName></Owner>
  <Buckets>
{bucket_xml}  </Buckets>
</ListAllMyBucketsResult>"#,
        owner = xml_escape(owner),
        bucket_xml = bucket_xml,
    )
}

// ── ListObjectsV2 ─────────────────────────────────────────────────────────────

pub struct ObjectListItem {
    pub key: String,
    pub last_modified: DateTime<Utc>,
    pub etag: String,
    pub size: u64,
    pub storage_class: String,
    pub owner_id: String,
}

pub struct ListObjectsV2Result {
    pub name: String,
    pub prefix: String,
    pub delimiter: Option<String>,
    pub max_keys: u32,
    pub key_count: u32,
    pub is_truncated: bool,
    pub next_continuation_token: Option<String>,
    pub contents: Vec<ObjectListItem>,
    pub common_prefixes: Vec<String>,
}

pub fn list_objects_v2(r: &ListObjectsV2Result) -> String {
    let mut contents = String::new();
    for obj in &r.contents {
        contents.push_str(&format!(
            r#"  <Contents>
    <Key>{key}</Key>
    <LastModified>{lm}</LastModified>
    <ETag>&quot;{etag}&quot;</ETag>
    <Size>{size}</Size>
    <StorageClass>{sc}</StorageClass>
    <Owner><ID>{owner}</ID></Owner>
  </Contents>
"#,
            key = xml_escape(&obj.key),
            lm = s3_date(&obj.last_modified),
            etag = xml_escape(&obj.etag),
            size = obj.size,
            sc = xml_escape(&obj.storage_class),
            owner = xml_escape(&obj.owner_id),
        ));
    }
    let mut common = String::new();
    for cp in &r.common_prefixes {
        common.push_str(&format!(
            "  <CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>\n",
            xml_escape(cp)
        ));
    }
    let truncated = if r.is_truncated { "true" } else { "false" };
    let next_token = r
        .next_continuation_token
        .as_deref()
        .map(|t| {
            format!(
                "<NextContinuationToken>{}</NextContinuationToken>",
                xml_escape(t)
            )
        })
        .unwrap_or_default();
    let delimiter = r
        .delimiter
        .as_deref()
        .map(|d| format!("<Delimiter>{}</Delimiter>", xml_escape(d)))
        .unwrap_or_default();

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>{name}</Name>
  <Prefix>{prefix}</Prefix>
  {delimiter}
  <MaxKeys>{max_keys}</MaxKeys>
  <KeyCount>{key_count}</KeyCount>
  <IsTruncated>{truncated}</IsTruncated>
  {next_token}
{contents}{common}</ListBucketResult>"#,
        name = xml_escape(&r.name),
        prefix = xml_escape(&r.prefix),
        delimiter = delimiter,
        max_keys = r.max_keys,
        key_count = r.key_count,
        truncated = truncated,
        next_token = next_token,
        contents = contents,
        common = common,
    )
}

// ── ListObjectVersions ────────────────────────────────────────────────────────

pub struct VersionItem {
    pub key: String,
    pub version_id: String,
    pub is_latest: bool,
    pub last_modified: DateTime<Utc>,
    pub etag: String,
    pub size: u64,
    pub storage_class: String,
    pub is_delete_marker: bool,
}

pub fn list_object_versions(bucket: &str, prefix: &str, versions: &[VersionItem]) -> String {
    let mut entries = String::new();
    for v in versions {
        if v.is_delete_marker {
            entries.push_str(&format!(
                r#"  <DeleteMarker>
    <Key>{key}</Key>
    <VersionId>{vid}</VersionId>
    <IsLatest>{latest}</IsLatest>
    <LastModified>{lm}</LastModified>
  </DeleteMarker>
"#,
                key = xml_escape(&v.key),
                vid = xml_escape(&v.version_id),
                latest = v.is_latest,
                lm = s3_date(&v.last_modified),
            ));
        } else {
            entries.push_str(&format!(
                r#"  <Version>
    <Key>{key}</Key>
    <VersionId>{vid}</VersionId>
    <IsLatest>{latest}</IsLatest>
    <LastModified>{lm}</LastModified>
    <ETag>&quot;{etag}&quot;</ETag>
    <Size>{size}</Size>
    <StorageClass>{sc}</StorageClass>
  </Version>
"#,
                key = xml_escape(&v.key),
                vid = xml_escape(&v.version_id),
                latest = v.is_latest,
                lm = s3_date(&v.last_modified),
                etag = xml_escape(&v.etag),
                size = v.size,
                sc = xml_escape(&v.storage_class),
            ));
        }
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ListVersionsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>{name}</Name>
  <Prefix>{prefix}</Prefix>
  <IsTruncated>false</IsTruncated>
{entries}</ListVersionsResult>"#,
        name = xml_escape(bucket),
        prefix = xml_escape(prefix),
        entries = entries,
    )
}

// ── InitiateMultipartUpload ───────────────────────────────────────────────────

pub fn initiate_multipart_upload(bucket: &str, key: &str, upload_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Bucket>{bucket}</Bucket>
  <Key>{key}</Key>
  <UploadId>{upload_id}</UploadId>
</InitiateMultipartUploadResult>"#,
        bucket = xml_escape(bucket),
        key = xml_escape(key),
        upload_id = xml_escape(upload_id),
    )
}

// ── CompleteMultipartUpload ───────────────────────────────────────────────────

pub fn complete_multipart_upload(
    location: &str,
    bucket: &str,
    key: &str,
    etag: &str,
    version_id: Option<&str>,
) -> String {
    let vid = version_id
        .map(|v| format!("<VersionId>{}</VersionId>", xml_escape(v)))
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Location>{location}</Location>
  <Bucket>{bucket}</Bucket>
  <Key>{key}</Key>
  <ETag>&quot;{etag}&quot;</ETag>
  {vid}
</CompleteMultipartUploadResult>"#,
        location = xml_escape(location),
        bucket = xml_escape(bucket),
        key = xml_escape(key),
        etag = xml_escape(etag),
        vid = vid,
    )
}

// ── ListMultipartUploads ──────────────────────────────────────────────────────

pub struct MultipartUploadItem {
    pub key: String,
    pub upload_id: String,
    pub initiated: DateTime<Utc>,
    pub owner_id: String,
}

pub fn list_multipart_uploads(bucket: &str, uploads: &[MultipartUploadItem]) -> String {
    let mut entries = String::new();
    for u in uploads {
        entries.push_str(&format!(
            r#"  <Upload>
    <Key>{key}</Key>
    <UploadId>{uid}</UploadId>
    <Initiator><ID>{owner}</ID></Initiator>
    <Owner><ID>{owner}</ID></Owner>
    <StorageClass>STANDARD</StorageClass>
    <Initiated>{initiated}</Initiated>
  </Upload>
"#,
            key = xml_escape(&u.key),
            uid = xml_escape(&u.upload_id),
            owner = xml_escape(&u.owner_id),
            initiated = s3_date(&u.initiated),
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ListMultipartUploadsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Bucket>{bucket}</Bucket>
  <IsTruncated>false</IsTruncated>
{entries}</ListMultipartUploadsResult>"#,
        bucket = xml_escape(bucket),
        entries = entries,
    )
}

// ── ListParts ─────────────────────────────────────────────────────────────────

pub struct PartItem {
    pub part_number: u32,
    pub last_modified: DateTime<Utc>,
    pub etag: String,
    pub size: u64,
}

pub fn list_parts(bucket: &str, key: &str, upload_id: &str, parts: &[PartItem]) -> String {
    let mut entries = String::new();
    for p in parts {
        entries.push_str(&format!(
            r#"  <Part>
    <PartNumber>{pn}</PartNumber>
    <LastModified>{lm}</LastModified>
    <ETag>&quot;{etag}&quot;</ETag>
    <Size>{size}</Size>
  </Part>
"#,
            pn = p.part_number,
            lm = s3_date(&p.last_modified),
            etag = xml_escape(&p.etag),
            size = p.size,
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ListPartsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Bucket>{bucket}</Bucket>
  <Key>{key}</Key>
  <UploadId>{uid}</UploadId>
  <IsTruncated>false</IsTruncated>
{entries}</ListPartsResult>"#,
        bucket = xml_escape(bucket),
        key = xml_escape(key),
        uid = xml_escape(upload_id),
        entries = entries,
    )
}

// ── Delete Objects ────────────────────────────────────────────────────────────

pub struct DeleteResult {
    pub deleted: Vec<String>,
    pub errors: Vec<(String, String, String)>, // (key, code, message)
}

pub fn delete_objects_result(result: &DeleteResult) -> String {
    let mut entries = String::new();
    for key in &result.deleted {
        entries.push_str(&format!(
            "  <Deleted><Key>{}</Key></Deleted>\n",
            xml_escape(key)
        ));
    }
    for (key, code, msg) in &result.errors {
        entries.push_str(&format!(
            "  <Error><Key>{}</Key><Code>{}</Code><Message>{}</Message></Error>\n",
            xml_escape(key),
            xml_escape(code),
            xml_escape(msg),
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DeleteResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
{entries}</DeleteResult>"#,
        entries = entries,
    )
}

// ── Versioning ────────────────────────────────────────────────────────────────

pub fn get_bucket_versioning(state: &str) -> String {
    if state == "Disabled" {
        return r#"<?xml version="1.0" encoding="UTF-8"?>
<VersioningConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"/>
"#
        .to_string();
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<VersioningConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Status>{state}</Status>
</VersioningConfiguration>"#,
        state = xml_escape(state),
    )
}

// ── CopyObject ────────────────────────────────────────────────────────────────

pub fn copy_object_result(etag: &str, last_modified: &DateTime<Utc>) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CopyObjectResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <LastModified>{lm}</LastModified>
  <ETag>&quot;{etag}&quot;</ETag>
</CopyObjectResult>"#,
        lm = s3_date(last_modified),
        etag = xml_escape(etag),
    )
}

// ── Tagging ───────────────────────────────────────────────────────────────────

pub fn get_object_tagging(tags: &std::collections::HashMap<String, String>) -> String {
    let mut entries = String::new();
    for (k, v) in tags {
        entries.push_str(&format!(
            "    <Tag><Key>{}</Key><Value>{}</Value></Tag>\n",
            xml_escape(k),
            xml_escape(v),
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <TagSet>
{entries}  </TagSet>
</Tagging>"#,
        entries = entries,
    )
}
