use crate::streaming::stats::stats::PartitionStats;
use crate::{
    slab::{
        Keyed,
        consumer_groups::ConsumerGroups,
        partitions::{PARTITIONS_CAPACITY, Partitions},
    },
    streaming::partitions::partition::ConsumerOffset,
};
use ahash::AHashMap;
use dashmap::DashMap;
use iggy_common::{CompressionAlgorithm, IggyExpiry, IggyTimestamp, MaxTopicSize};
use slab::Slab;
use std::sync::Arc;

#[derive(Default, Debug)]
pub struct Topic {
    id: usize,
    // TODO: This property should be removed, we won't use it in our clustering impl.
    replication_factor: u8,
    name: String,
    created_at: IggyTimestamp,
    message_expiry: IggyExpiry,
    compression_algorithm: CompressionAlgorithm,
    max_topic_size: MaxTopicSize,

    partitions: Partitions,
    consumer_groups: ConsumerGroups,

    consumer_offsets: Slab<AHashMap<usize, ConsumerOffset>>,
    consumer_group_offsets: Slab<AHashMap<usize, ConsumerOffset>>,
}

impl Topic {
    pub fn new(
        name: String,
        replication_factor: u8,
        message_expiry: IggyExpiry,
        compression: CompressionAlgorithm,
        max_topic_size: MaxTopicSize,
    ) -> Self {
        Self {
            id: 0,
            name,
            created_at: IggyTimestamp::now(),
            replication_factor,
            message_expiry,
            compression_algorithm: compression,
            max_topic_size,
            partitions: Partitions::default(),
            consumer_groups: ConsumerGroups::default(),
            consumer_offsets: Slab::with_capacity(PARTITIONS_CAPACITY),
            consumer_group_offsets: Slab::with_capacity(PARTITIONS_CAPACITY),
        }
    }

    pub fn invoke<T>(&self, f: impl FnOnce(&Self) -> T) -> T {
        f(self)
    }

    pub fn invoke_mut<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        f(self)
    }

    pub async fn invoke_async<T>(&self, f: impl AsyncFnOnce(&Self) -> T) -> T {
        f(self).await
    }

    pub fn with_partition_stats_mut<T>(
        &mut self,
        f: impl FnOnce(&mut Slab<Arc<PartitionStats>>) -> T,
    ) -> T {
        self.partitions.with_stats_mut(f)
    }

    pub fn message_expiry(&self) -> IggyExpiry {
        self.message_expiry
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn set_compression(&mut self, compression: CompressionAlgorithm) {
        self.compression_algorithm = compression;
    }

    pub fn set_message_expiry(&mut self, message_expiry: IggyExpiry) {
        self.message_expiry = message_expiry;
    }

    pub fn set_max_topic_size(&mut self, max_topic_size: MaxTopicSize) {
        self.max_topic_size = max_topic_size;
    }

    pub fn set_replication_factor(&mut self, replication_factor: u8) {
        self.replication_factor = replication_factor;
    }

    pub fn partitions(&self) -> &Partitions {
        &self.partitions
    }

    pub fn partitions_mut(&mut self) -> &mut Partitions {
        &mut self.partitions
    }

    pub fn consumer_groups(&self) -> &ConsumerGroups {
        &self.consumer_groups
    }

    pub fn consumer_groups_mut(&mut self) -> &mut ConsumerGroups {
        &mut self.consumer_groups
    }

    pub fn consumer_offsets(&self) -> &Slab<AHashMap<usize, ConsumerOffset>> {
        &self.consumer_offsets
    }

    pub fn consumer_offsets_mut(&mut self) -> &mut Slab<AHashMap<usize, ConsumerOffset>> {
        &mut self.consumer_offsets
    }

    pub fn consumer_group_offsets(&self) -> &Slab<AHashMap<usize, ConsumerOffset>> {
        &self.consumer_group_offsets
    }

    pub fn consumer_group_offsets_mut(&mut self) -> &mut Slab<AHashMap<usize, ConsumerOffset>> {
        &mut self.consumer_group_offsets
    }

    pub fn insert_into(self, container: &mut Slab<Self>) -> usize {
        let idx = container.insert(self);
        let topic = &mut container[idx];
        topic.id = idx;
        idx
    }
}

impl Keyed for Topic {
    type Key = String;

    fn key(&self) -> &Self::Key {
        &self.name
    }
}
