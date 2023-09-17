// Digital garden visitor counter
// A simple visitor counter for digital gardens that runs as an AWS Lambda function.
// Copyright (C) 2023 John DiSanti.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! Count and recent visitor storage in DynamoDB.
//!
//! This module provides an abstraction over the DynamoDB table used to store
//! counts and recent visitors.
//!
//! The Lambda is able to store multiple counters in a single DynamoDB table,
//! and a single item is used for each counter. The item's key is the counter
//! name, and the item has two attributes: `count` and `value`. The `count`
//! is just the current counter value, and `value` is a CBOR encoded list
//! of recent visitors. Only a 32-bit hash of the visitor's IP and user agent,
//! and the time they were last seen are stored.
//!
//! The 400 KB maximum item size is taken into account, and the recent visitors
//! list is culled if it starts getting too long. Additionally, visitors that
//! haven't been seen in a while are removed from the list.
//!
//! Optimistic locking via conditional expressions is used to prevent concurrent
//! Lambda invocations from overwriting each other's updates. If a Lambda invocation
//! fails to update the item due to the condition failing, it will reload the current count
//! and reapply its update up to 5 times before giving up.

use crate::request_info::RequestInfo;
use aws_config::{retry::RetryConfig, timeout::TimeoutConfig};
use aws_sdk_dynamodb::{
    error::{BoxError, SdkError},
    operation::{
        get_item::{builders::GetItemInputBuilder, GetItemError, GetItemInput, GetItemOutput},
        put_item::{builders::PutItemInputBuilder, PutItemError, PutItemInput, PutItemOutput},
    },
    primitives::Blob,
    types::AttributeValue,
    Client,
};
use md5::{Digest, Md5};
use std::{
    future::Future,
    mem::size_of,
    pin::Pin,
    time::{Duration, SystemTime},
};

const MAX_ATTEMPTS_FOR_OPTIMISTIC_LOCKING: usize = 5;
const DYNAMO_MAX_ITEM_SIZE_BYTES: usize = 400_000;
const RESERVED_NON_VALUE_SIZE_BYTES: usize = 1024;
const SIZE_SINGLE_VISITOR_BYTES: usize = 15;
const MAX_RECENT_VISITORS: usize =
    (DYNAMO_MAX_ITEM_SIZE_BYTES - RESERVED_NON_VALUE_SIZE_BYTES) / SIZE_SINGLE_VISITOR_BYTES;

/// This value was chosen so that the stored timestamp could be 32-bits
/// and still work well into the future.
const TIMESTAMP_OFFSET: u64 = 1_690_000_000;

/// How long a visitor is kept in the recent visitors list before being pruned.
const RECENT_CUTOFF: Duration = Duration::from_secs(7200); // 2 hours

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Trait representing the only operations we use in the DynamoDB client.
///
/// This is a trait so that the Dynamo calls can be trivially mocked in unit tests.
trait Dynamo {
    /// Get an item from DynamoDB.
    fn get_item(
        &self,
        input: GetItemInputBuilder,
    ) -> BoxFuture<Result<GetItemOutput, SdkError<GetItemError>>>;

    /// Put an item to DynamoDB.
    fn put_item(
        &self,
        input: PutItemInputBuilder,
    ) -> BoxFuture<Result<PutItemOutput, SdkError<PutItemError>>>;
}

/// A client that can be switched between real and fake modes for testing.
#[derive(Clone)]
enum DynamoClient {
    /// The real DynamoDB client.
    Real(Client),
    /// A fake client with mocked calls for testing.
    #[cfg(test)]
    Fake(std::sync::Arc<dyn Dynamo>),
}

impl Dynamo for DynamoClient {
    fn get_item(
        &self,
        input: GetItemInputBuilder,
    ) -> BoxFuture<Result<GetItemOutput, SdkError<GetItemError>>> {
        match self {
            Self::Real(client) => {
                let client = client.clone();
                Box::pin(async move { input.send_with(&client).await })
            }
            #[cfg(test)]
            Self::Fake(fake) => fake.get_item(input),
        }
    }

    fn put_item(
        &self,
        input: PutItemInputBuilder,
    ) -> BoxFuture<Result<PutItemOutput, SdkError<PutItemError>>> {
        match self {
            Self::Real(client) => {
                let client = client.clone();
                Box::pin(async move { input.send_with(&client).await })
            }
            #[cfg(test)]
            Self::Fake(fake) => fake.put_item(input),
        }
    }
}

/// The stored representation of a visitor.
#[derive(Copy, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
struct StoredVisitor {
    #[serde(rename = "g")]
    tag: u32,
    /// Seconds since `TIMESTAMP_OFFSET`.
    #[serde(rename = "t")]
    last_seen: u32,
}

impl From<Visitor> for StoredVisitor {
    fn from(value: Visitor) -> Self {
        StoredVisitor {
            tag: value.tag,
            last_seen: u32::try_from(
                value
                    .last_seen
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("unix epoch before last seen")
                    .as_secs()
                    .checked_sub(TIMESTAMP_OFFSET)
                    .expect("last_seen will always be after TIMESTAMP_OFFSET"),
            )
            .expect("last_seen will fit in a u32 until something like year 2140"),
        }
    }
}

/// A visitor to the site.
#[derive(Copy, Clone, Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Visitor {
    /// Hashed source IP and user agent.
    pub tag: u32,
    /// Time last seen.
    pub last_seen: SystemTime,
}

#[cfg(test)]
impl Visitor {
    fn new(tag: u32, last_seen: SystemTime) -> Self {
        Self { tag, last_seen }
    }
}

impl From<StoredVisitor> for Visitor {
    fn from(value: StoredVisitor) -> Self {
        Visitor {
            tag: value.tag,
            last_seen: SystemTime::UNIX_EPOCH
                + Duration::from_secs(TIMESTAMP_OFFSET + value.last_seen as u64),
        }
    }
}

impl From<&RequestInfo> for Visitor {
    fn from(value: &RequestInfo) -> Self {
        // Use the first 32-bits of an MD5 hash of the source IP and user agent to
        // roughly track uniqueness without storing any identifying information.
        let mut hasher = Md5::new();
        hasher.update(&value.source_ip);
        hasher.update(&value.user_agent);
        let hash = &hasher.finalize()[0..size_of::<u32>()];
        let tag = u32_from_ne_bytes(hash);

        Visitor {
            tag,
            last_seen: SystemTime::now(),
        }
    }
}

/// Stored representation of a count entry. This becomes the value of the
/// "value" attribute in DynamoDB, and is stored as a CBOR blob.
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredCountEntry {
    #[serde(rename = "v")]
    recent_visitors: Vec<StoredVisitor>,
}

impl StoredCountEntry {
    fn to_cbor(&self) -> Result<Vec<u8>, BoxError> {
        let mut output = Vec::new();
        ciborium::into_writer(self, &mut output)?;
        Ok(output)
    }

    fn from_cbor(cbor: &[u8]) -> Result<Self, BoxError> {
        Ok(ciborium::from_reader(cbor)?)
    }
}

impl From<&CountEntry> for StoredCountEntry {
    fn from(value: &CountEntry) -> Self {
        StoredCountEntry {
            recent_visitors: value
                .recent_visitors
                .iter()
                .copied()
                .map(StoredVisitor::from)
                .collect(),
        }
    }
}

/// A count, and the most recent visitors contributing to that count.
#[derive(Debug, Default)]
pub struct CountEntry {
    pub count: u64,
    pub recent_visitors: Vec<Visitor>,
}

impl From<StoredCountEntry> for CountEntry {
    fn from(value: StoredCountEntry) -> Self {
        CountEntry {
            count: 0,
            recent_visitors: value
                .recent_visitors
                .into_iter()
                .map(Visitor::from)
                .collect(),
        }
    }
}

/// An abstraction over count storage in DynamoDB.
#[derive(Clone)]
pub struct Store {
    client: DynamoClient,
    table_name: String,
}

impl Store {
    /// Creates a new `Store` with the given table name.
    pub async fn new(table_name: impl Into<String>) -> Self {
        // The SDK has really high default connect/read timeouts for this use-case since
        // DynamoDB usually responds in less than 10 milliseconds. It also doesn't have
        // a default operation timeout, which means if the server connects and responds
        // with partial content and then hangs, the Lambda could hang indefinitely.
        // Therefore, change the configuration to reduce overall wait time and avoid an
        // infinitely hanging Lambda.
        let connect_read_timeout = Duration::from_millis(100);
        let config = aws_config::from_env()
            .timeout_config(
                TimeoutConfig::builder()
                    .connect_timeout(connect_read_timeout)
                    .read_timeout(connect_read_timeout)
                    .operation_timeout(Duration::from_millis(200))
                    .build(),
            )
            // Reduce the number of retry attempts to avoid spending too much time.
            .retry_config(RetryConfig::standard().with_max_attempts(2))
            .load()
            .await;
        Self {
            client: DynamoClient::Real(Client::new(&config)),
            table_name: table_name.into(),
        }
    }

    /// Creates a `Store` with a mocked DynamoDB client for testing.
    #[cfg(test)]
    fn fake(table_name: impl Into<String>, dynamo: impl Dynamo + 'static) -> Self {
        Self {
            client: DynamoClient::Fake(std::sync::Arc::new(dynamo)),
            table_name: table_name.into(),
        }
    }

    /// Loads a count entry with the given name from DynamoDB.
    async fn get_count_entry(&self, name: &str) -> Result<Option<CountEntry>, BoxError> {
        // Load the row from DynamoDB.
        let input = GetItemInput::builder()
            .table_name(&self.table_name)
            .key("key", AttributeValue::S(name.into()));
        let output = self.client.get_item(input).await?;
        if output.item.is_none() {
            return Ok(None);
        }

        // Convert the row's attributes back into a CountEntry.
        let item = output.item.as_ref();
        let count = item
            .and_then(|item| item.get("count"))
            .map(|value| {
                value
                    .as_n()
                    .map_err(|_| "count is not a number")
                    .and_then(|n| n.parse::<u64>().map_err(|_| "failed to parse count"))
            })
            .transpose()?
            .ok_or("item was missing a count attribute")?;
        let value = item
            .and_then(|item| item.get("value"))
            .map(|attr| {
                attr.as_b()
                    .map_err(|_| BoxError::from("value was not a blob"))
                    .and_then(|b| StoredCountEntry::from_cbor(b.as_ref()))
            })
            .transpose()?
            .ok_or("item was missing a value attribute")?;
        let mut entry = CountEntry::from(value);
        entry.count = count;
        Ok(Some(entry))
    }

    /// Creates a new count entry.
    ///
    /// Returns true if the creationg succeeded, and false if another invocation
    /// created the entry before this one did. In that case, the caller should
    /// retry as an update instead of a creation.
    async fn try_put_new_count_entry(
        &self,
        name: &str,
        visitor: Visitor,
    ) -> Result<bool, BoxError> {
        let value = Blob::new(
            StoredCountEntry {
                recent_visitors: vec![StoredVisitor::from(visitor)],
            }
            .to_cbor()?,
        );
        let input = PutItemInput::builder()
            .table_name(&self.table_name)
            .condition_expression("attribute_not_exists(#k)")
            .expression_attribute_names("#k", "key")
            .item("key", AttributeValue::S(name.into()))
            .item("count", AttributeValue::N("1".into()))
            .item("value", AttributeValue::B(value));
        let result = self.client.put_item(input).await;
        match result {
            Ok(_) => Ok(true),
            Err(err) => match err.into_service_error() {
                PutItemError::ConditionalCheckFailedException(_) => Ok(false),
                e => Err(e.into()),
            },
        }
    }

    /// Try to update an existing count entry.
    ///
    /// Returns true if the update succeeded, and false if there was a conditional check failure.
    /// The conditional check failure indicates the update should be retried.
    async fn try_put_count_entry(
        &self,
        name: &str,
        initial_count: u64,
        entry: &CountEntry,
    ) -> Result<bool, BoxError> {
        let value = Blob::new(StoredCountEntry::from(entry).to_cbor()?);
        let input = PutItemInput::builder()
            .table_name(&self.table_name)
            .condition_expression("#c = :count")
            .expression_attribute_names("#c", "count")
            .expression_attribute_values(":count", AttributeValue::N(initial_count.to_string()))
            .item("key", AttributeValue::S(name.into()))
            .item("count", AttributeValue::N(entry.count.to_string()))
            .item("value", AttributeValue::B(value));
        let result = self.client.put_item(input).await;
        match result {
            Ok(_) => Ok(true),
            Err(err) => match err.into_service_error() {
                PutItemError::ConditionalCheckFailedException(_) => Ok(false),
                e => Err(e.into()),
            },
        }
    }

    /// Find the given visitor in the recent visitor list by tag, and return a mutable reference to it.
    fn find_recent_mut(
        count_entry: &mut CountEntry,
        visitor: Visitor,
        now: SystemTime,
    ) -> Option<&mut Visitor> {
        count_entry
            .recent_visitors
            .iter_mut()
            .find(|v| v.tag == visitor.tag)
            .filter(|v| {
                now.duration_since(v.last_seen)
                    .expect("now is after last_seen")
                    < RECENT_CUTOFF
            })
    }

    /// Removes visitors from the recent visitors list that haven't been seen recently,
    /// or the oldest visitors if the list is getting too long.
    fn prune_visitors(count_entry: &mut CountEntry, now: SystemTime, max_recent: usize) {
        let visitors = std::mem::take(&mut count_entry.recent_visitors);
        count_entry.recent_visitors = visitors
            .into_iter()
            .filter(|v| {
                now.duration_since(v.last_seen)
                    .expect("now is after last_seen")
                    < RECENT_CUTOFF
            })
            .collect();
        if count_entry.recent_visitors.len() > max_recent {
            // Sort descending by last seen time.
            count_entry
                .recent_visitors
                .sort_by(|left, right| right.last_seen.cmp(&left.last_seen));
            // Cull the oldest by truncating.
            count_entry.recent_visitors.truncate(max_recent);
        }
    }

    /// Increment the number of visitors (if this visitor is recently unique), and return the count.
    pub async fn maybe_increment_visitors(
        &self,
        visitor: Visitor,
        name: &str,
        now: SystemTime,
    ) -> Result<usize, BoxError> {
        // Looping since we're using optimistic locking. There is a chance another simultaneous execution
        // of this Lambda tries to update the row at the same time. If that happens, keep trying until
        // it works, or until we get to max attempts.
        let mut attempt = 0;
        while attempt < MAX_ATTEMPTS_FOR_OPTIMISTIC_LOCKING {
            let mut count_entry = self.get_count_entry(name).await?;
            if let Some(count_entry) = &mut count_entry {
                let initial_count = count_entry.count;

                // If the visitor has been seen recently, then just update the last seen time.
                // Otherwise, add them to the recent list and increment the count.
                if let Some(recent) = Self::find_recent_mut(count_entry, visitor, now) {
                    recent.last_seen = now;
                } else {
                    count_entry.recent_visitors.push(visitor);
                    count_entry.count += 1;
                }

                // Prune old visitors
                Self::prune_visitors(count_entry, now, MAX_RECENT_VISITORS);

                // Update the entry in DynamoDB.
                if self
                    .try_put_count_entry(name, initial_count, count_entry)
                    .await?
                {
                    return Ok(count_entry.count as usize);
                } else {
                    attempt += 1;
                    continue;
                }
            } else {
                // Try to create a new entry in DynamoDB if there was no entry.
                if self.try_put_new_count_entry(name, visitor).await? {
                    return Ok(1);
                } else {
                    attempt += 1;
                    continue;
                }
            }
        }
        Err("max attempts for optimistic locking exceeded".into())
    }
}

/// Convert a slice of bytes into a single u32, assuming the bytes are in native endian format.
fn u32_from_ne_bytes(bytes: &[u8]) -> u32 {
    let mut buf = [0; size_of::<u32>()];
    buf.copy_from_slice(bytes);
    unsafe { std::mem::transmute(buf) }
}

#[cfg(test)]
mod conversion_tests {
    use super::*;

    fn big_endian() -> bool {
        let x: u32 = 1;
        let x_bytes: [u8; size_of::<u32>()] = unsafe { std::mem::transmute(x) };
        x_bytes[0] != 1
    }

    #[test]
    fn test_u32_from_ne_bytes() {
        let bytes = if big_endian() {
            [0x01, 0x02, 0x03, 0x04]
        } else {
            [0x04, 0x03, 0x02, 0x01]
        };
        let result = super::u32_from_ne_bytes(&bytes);
        assert_eq!(result, 16909060);
    }

    #[test]
    fn test_from() {
        let visitor = Visitor::from(&RequestInfo {
            user_agent: "test".to_string(),
            source_ip: "127.0.0.1".to_string(),
        });
        assert_eq!(1600273645, visitor.tag);

        let visitor = Visitor::from(&RequestInfo {
            user_agent: "test2".to_string(),
            source_ip: "127.0.0.1".to_string(),
        });
        assert_eq!(508621390, visitor.tag);

        let visitor = Visitor::from(&RequestInfo {
            user_agent: "testv6".to_string(),
            source_ip: "0:0:0:0:0:0:0:1".to_string(),
        });
        assert_eq!(4102698867, visitor.tag);
    }

    #[test]
    fn visitor_stored_visitor_round_trip() {
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(TIMESTAMP_OFFSET + 1000);
        let visitor = Visitor::new(1234, time);
        let stored = StoredVisitor::from(visitor);
        assert_eq!(1234, stored.tag);
        assert_eq!(1000, stored.last_seen);

        let visitor_again = Visitor::from(stored);
        assert_eq!(1234, visitor_again.tag);
        assert_eq!(time, visitor_again.last_seen);
    }

    #[test]
    fn count_entry_round_trip() {
        let time1 = SystemTime::UNIX_EPOCH + Duration::from_secs(TIMESTAMP_OFFSET);
        let time2 = time1 + Duration::from_secs(1000);
        let entry = CountEntry {
            count: 1234,
            recent_visitors: vec![Visitor::new(1, time1), Visitor::new(2, time2)],
        };

        let stored = StoredCountEntry::from(&entry);
        assert_eq!(2, stored.recent_visitors.len());
        assert_eq!(1, stored.recent_visitors[0].tag);
        assert_eq!(0, stored.recent_visitors[0].last_seen);
        assert_eq!(2, stored.recent_visitors[1].tag);
        assert_eq!(1000, stored.recent_visitors[1].last_seen);

        let entry_again = CountEntry::from(stored);
        assert_eq!(
            0, entry_again.count,
            "count isn't stored on the stored entry"
        );
        assert_eq!(2, entry_again.recent_visitors.len());
        assert_eq!(1, entry_again.recent_visitors[0].tag);
        assert_eq!(time1, entry_again.recent_visitors[0].last_seen);
        assert_eq!(2, entry_again.recent_visitors[1].tag);
        assert_eq!(time2, entry_again.recent_visitors[1].last_seen);
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;
    use aws_sdk_dynamodb::types::error::ConditionalCheckFailedException;
    use aws_smithy_http::body::SdkBody;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    impl StoredVisitor {
        fn new(tag: u32, last_seen: u32) -> Self {
            Self { tag, last_seen }
        }
    }

    fn system_time(offset: u32) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(TIMESTAMP_OFFSET + offset as u64)
    }

    fn output(count: u64, recent_visitors: Vec<StoredVisitor>) -> GetItemOutput {
        let value = StoredCountEntry { recent_visitors }.to_cbor().unwrap();
        GetItemOutput::builder()
            .item("key", AttributeValue::S("default".into()))
            .item("count", AttributeValue::N(count.to_string()))
            .item("value", AttributeValue::B(Blob::new(value)))
            .build()
    }

    #[track_caller]
    fn assert_get(get: &GetItemInput, name: &str) {
        assert_eq!("test", get.table_name.as_ref().unwrap(), "wrong table name");
        assert_eq!(
            &AttributeValue::S(name.into()),
            get.key.as_ref().unwrap().get("key").unwrap(),
            "wrong key value"
        );
    }

    #[track_caller]
    fn assert_put(
        put: &PutItemInput,
        name: &str,
        initial_count: u64,
        new_count: u64,
        recent_visitors: &[StoredVisitor],
    ) {
        assert_eq!("test", put.table_name.as_ref().unwrap(), "wrong table name");
        assert_eq!(
            "#c = :count",
            put.condition_expression.as_ref().unwrap(),
            "wrong condition expression"
        );
        assert_eq!(
            "count",
            put.expression_attribute_names
                .as_ref()
                .unwrap()
                .get("#c")
                .unwrap(),
            "wrong expression attribute name"
        );
        assert_eq!(
            &AttributeValue::N(initial_count.to_string()),
            put.expression_attribute_values
                .as_ref()
                .unwrap()
                .get(":count")
                .unwrap(),
            "the count will only get incremented if it is its previous value",
        );

        let item = put.item.as_ref().unwrap();
        assert_eq!(
            &AttributeValue::S(name.into()),
            item.get("key").unwrap(),
            "wrong key value"
        );
        assert_eq!(
            &AttributeValue::N(new_count.to_string()),
            item.get("count").unwrap(),
            "wrong count value"
        );

        let value =
            StoredCountEntry::from_cbor(item.get("value").unwrap().as_b().unwrap().as_ref())
                .unwrap();
        assert_eq!(
            recent_visitors, value.recent_visitors,
            "incorrect recent visitors"
        );
    }

    macro_rules! fake_dynamo {
        (
            get($get_input:ident) => { $($get:tt)+ },
            put($put_input:ident, $attempt:ident) => { $($put:tt)+ },
        ) => {{
            struct Fake { #[allow(unused)] attempt: Arc<AtomicUsize> }
            impl Dynamo for Fake {
                fn get_item(
                    &self,
                    builder: GetItemInputBuilder,
                ) -> BoxFuture<Result<GetItemOutput, SdkError<GetItemError>>> {
                    Box::pin(async {
                        let $get_input = builder;
                        $($get)+
                    })
                }

                fn put_item(
                    &self,
                    builder: PutItemInputBuilder,
                ) -> BoxFuture<Result<PutItemOutput, SdkError<PutItemError>>> {
                    let _attempt = self.attempt.clone();
                    Box::pin(async move {
                        let $put_input = builder;
                        let $attempt = _attempt;
                        $($put)+
                    })
                }
            }
            Store::fake("test", Fake { attempt: Arc::new(AtomicUsize::new(0)) })
        }};
    }

    #[tokio::test]
    async fn create_item_when_not_existing() {
        let store = fake_dynamo!(
            get(input) => {
                // Verify the input to the DynamoDB GetItem call.
                assert_get(&input.build().unwrap(), "default");

                // Respond with an empty output, indicating the item doesn't exist.
                Ok(GetItemOutput::builder().build())
            },
            put(input, _attempt) => {
                // Verify the input to the DynamoDB PutItem call.
                let input = input.build().unwrap();
                assert_eq!("test", input.table_name.as_ref().unwrap(), "wrong table name");
                assert_eq!("attribute_not_exists(#k)", input.condition_expression.as_ref().unwrap(), "wrong condition expression");
                assert_eq!("key", input.expression_attribute_names.as_ref().unwrap().get("#k").unwrap(), "wrong expression attribute name");
                assert_eq!(None, input.expression_attribute_values.as_ref(), "there shouldn't be expression attrs");

                let item = input.item.as_ref().unwrap();
                assert_eq!(&AttributeValue::S("default".into()), item.get("key").unwrap(), "wrong key value");
                assert_eq!(&AttributeValue::N(1.to_string()), item.get("count").unwrap(), "wrong count value");

                let value = StoredCountEntry::from_cbor(item.get("value").unwrap().as_b().unwrap().as_ref()).unwrap();
                assert_eq!(
                    &[StoredVisitor::new(1, 1000)][..], value.recent_visitors,
                    "incorrect recent visitors"
                );

                // Return an empty successful response.
                Ok(PutItemOutput::builder().build())
            },
        );

        // It should increment the counter when the visitor is not in the recent list.
        let now = system_time(1000);
        let result = store
            .maybe_increment_visitors(Visitor::new(1, now), "default", now)
            .await
            .unwrap();
        assert_eq!(1, result);
    }

    #[tokio::test]
    async fn increment_count_when_visitor_not_recent() {
        let store = fake_dynamo!(
            get(input) => {
                // Verify the input to the DynamoDB GetItem call.
                assert_get(&input.build().unwrap(), "default");

                // Respond with a stored count that has no recent visitors.
                Ok(output(1234, Vec::new()))
            },
            put(input, _attempt) => {
                // Verify the input to the DynamoDB PutItem call.
                assert_put(
                    &input.build().unwrap(),
                    "default",
                    1234,
                    // The count is incremented
                    1235,
                    // The visitor is inserted into the recent list with the current time.
                    &[StoredVisitor::new(1234, 1000)],
                );

                // Return an empty successful response.
                Ok(PutItemOutput::builder().build())
            },
        );

        // It should increment the counter when the visitor is not in the recent list.
        let now = system_time(1000);
        let result = store
            .maybe_increment_visitors(Visitor::new(1234, now), "default", now)
            .await
            .unwrap();
        assert_eq!(1235, result);
    }

    #[tokio::test]
    async fn return_existing_count_when_visitor_recent() {
        let store = fake_dynamo!(
            get(input) => {
                // Verify the input to the DynamoDB GetItem call.
                assert_get(&input.build().unwrap(), "default");

                // Respond with a stored count that has the visitor in the recent list.
                Ok(output(1234, vec![StoredVisitor::new(1, 1000)]))
            },
            put(input, _attempt) => {
                // Verify the input to the DynamoDB PutItem call.
                assert_put(
                    &input.build().unwrap(),
                    "default",
                    1234,
                    // The count is not incremented.
                    1234,
                    // The visitor's last seen time is updated to the current time.
                    &[StoredVisitor::new(1, 2000)],
                );

                // Return an empty successful response.
                Ok(PutItemOutput::builder().build())
            },
        );

        let result = store
            .maybe_increment_visitors(
                Visitor::new(1, system_time(2000)),
                "default",
                system_time(2000),
            )
            .await
            .unwrap();
        assert_eq!(1234, result);
    }

    #[tokio::test]
    async fn retry_when_optimistic_lock_fails() {
        let store = fake_dynamo! {
            get(input) => {
                // Verify the input to the DynamoDB GetItem call.
                assert_get(&input.build().unwrap(), "default");

                // Respond with a stored count that has the visitor in the recent list.
                Ok(output(1234, vec![StoredVisitor::new(1, 0)]))
            },
            put(input, attempt) => {
                // Verify the input to the DynamoDB PutItem call.
                assert_put(
                    &input.build().unwrap(),
                    "default",
                    1234,
                    // It should increment since the time is passed the recent cutoff.
                    1235,
                    // The time should be updated.
                    &[StoredVisitor::new(1, 7201)],
                );

                // On the first attempt, fail with a ConditionalCheckFailedException.
                // On the second attempt, succeed.
                if attempt.load(Ordering::Relaxed) == 0 {
                    attempt.store(1, Ordering::Relaxed);
                    Err(SdkError::service_error(
                        PutItemError::ConditionalCheckFailedException(
                            ConditionalCheckFailedException::builder().build(),
                        ),
                        http::Response::builder()
                            .status(123) // doesn't matter
                            .body(SdkBody::empty())
                            .unwrap(),
                    ))
                } else {
                    Ok(PutItemOutput::builder().build())
                }
            },
        };

        let time = system_time(RECENT_CUTOFF.as_secs() as u32 + 1);
        let result = store
            .maybe_increment_visitors(Visitor::new(1, time), "default", time)
            .await
            .unwrap();
        assert_eq!(1235, result);
    }

    #[tokio::test]
    async fn prune_old_visitors() {
        let store = fake_dynamo!(
            get(input) => {
                // Verify the input to the DynamoDB GetItem call.
                assert_get(&input.build().unwrap(), "default");

                // Respond with a stored count that has the visitor in the recent list.
                Ok(output(1234, vec![
                    StoredVisitor::new(1, 0),
                    StoredVisitor::new(2, 0),
                    StoredVisitor::new(3, 0),
                    StoredVisitor::new(4, 10_000)
                ]))
            },
            put(input, _attempt) => {
                // Verify the input to the DynamoDB PutItem call.
                assert_put(
                    &input.build().unwrap(),
                    "default",
                    1234,
                    // The count gets incremented because the visit time is after the recent cutoff.
                    1235,
                    // The visitor's last seen time is updated to the current time, and the older
                    // visitor entries are removed.
                    &[
                        StoredVisitor::new(4, 10_000),
                        StoredVisitor::new(1, 12_000),
                    ],
                );

                // Return an empty successful response.
                Ok(PutItemOutput::builder().build())
            },
        );

        let result = store
            .maybe_increment_visitors(
                Visitor::new(1, system_time(12_000)),
                "default",
                system_time(12_000),
            )
            .await
            .unwrap();
        assert_eq!(1235, result);
    }

    #[test]
    fn recents_list_size() {
        let mut entry = StoredCountEntry {
            recent_visitors: Vec::new(),
        };

        let empty_size = entry.to_cbor().unwrap().len();
        entry
            .recent_visitors
            .push(StoredVisitor::new(u32::MAX, u32::MAX));
        let single_size = entry.to_cbor().unwrap().len() - empty_size;
        assert_eq!(
            SIZE_SINGLE_VISITOR_BYTES, single_size,
            "update the constant if this fails"
        );

        entry.recent_visitors = vec![StoredVisitor::new(u32::MAX, u32::MAX); MAX_RECENT_VISITORS];
        let full_size = entry.to_cbor().unwrap().len();
        assert!(
            full_size <= DYNAMO_MAX_ITEM_SIZE_BYTES - RESERVED_NON_VALUE_SIZE_BYTES,
            "full size {full_size} should be less than or equal to {}",
            DYNAMO_MAX_ITEM_SIZE_BYTES - RESERVED_NON_VALUE_SIZE_BYTES
        );
    }

    #[test]
    fn prune_oldest_after_reaching_max_recents() {
        let mut entry = CountEntry {
            count: 1,
            recent_visitors: vec![
                Visitor::new(5, system_time(10)),
                Visitor::new(2, system_time(100)),
                Visitor::new(4, system_time(25)),
                Visitor::new(1, system_time(150)),
                Visitor::new(3, system_time(50)),
            ],
        };

        Store::prune_visitors(&mut entry, system_time(150), 3);
        assert_eq!(
            &[
                Visitor::new(1, system_time(150)),
                Visitor::new(2, system_time(100)),
                Visitor::new(3, system_time(50)),
            ][..],
            &entry.recent_visitors,
        );
    }
}
