use anyhow::Context;
use std::sync::{Arc, RwLock};

use colmeia_dat1_proto::*;

pub struct PeeredHypercore<Storage>
where
    Storage: random_access_storage::RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>>
        + std::fmt::Debug
        + Send
        + Sync,
{
    feed: Arc<RwLock<hypercore::Feed<Storage>>>,
    channel: u64,
    handshake: SimpleDatHandshake,
    // remote_bitfield: hypercore::bitfield::Bitfield,
    remote_length: usize,
}

impl<Storage> std::ops::Deref for PeeredHypercore<Storage>
where
    Storage: random_access_storage::RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>>
        + std::fmt::Debug
        + Send
        + Sync,
{
    type Target = Arc<RwLock<hypercore::Feed<Storage>>>;
    fn deref(&self) -> &Self::Target {
        &self.feed
    }
}

impl<Storage> PeeredHypercore<Storage>
where
    Storage: random_access_storage::RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>>
        + std::fmt::Debug
        + Send
        + Sync,
{
    pub fn new(channel: u64, feed: Arc<RwLock<hypercore::Feed<Storage>>>) -> Self {
        Self {
            feed,
            channel,
            handshake: SimpleDatHandshake::default(),
            // remote_bitfield: hypercore::bitfield::Bitfield::default(),
            remote_length: 0,
        }
    }
}

#[async_trait]
impl<Storage> DatProtocolEvents for PeeredHypercore<Storage>
where
    Storage: random_access_storage::RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>>
        + std::fmt::Debug
        + Send
        + Sync,
{
    type Err = anyhow::Error;

    async fn on_feed(
        &mut self,
        client: &mut Client,
        channel: u64,
        message: &proto::Feed,
    ) -> Result<(), Self::Err> {
        self.handshake.on_feed(client, channel, message).await?;
        Ok(())
    }

    async fn on_handshake(
        &mut self,
        client: &mut Client,
        channel: u64,
        message: &proto::Handshake,
    ) -> Result<(), Self::Err> {
        if channel != self.channel {
            return Err(anyhow::anyhow!("Wrong channel in use"));
        }
        self.handshake
            .on_handshake(client, channel, message)
            .await?;

        let mut message = proto::Want::new();
        message.set_start(0);
        message.set_length(0); // must be in sizes of 8192 bytes
        client.want(channel, &message).await?;

        Ok(())
    }

    // https://github.com/mafintosh/hypercore/blob/84990baa477491478f79b968123d2233eebeba76/lib/replicate.js#L109-L123
    async fn on_want(
        &mut self,
        client: &mut Client,
        channel: u64,
        message: &proto::Want,
    ) -> Result<(), Self::Err> {
        // We only reploy to multiple of 8192 in terms of offsets and lengths for want messages
        // since this is much easier for the bitfield, in terms of paging.
        if (message.get_start() & 8191 != 0) || (message.get_length() & 8191 != 0) {
            return Ok(());
        }

        let feed_length = self
            .feed
            .read()
            .map_err(|_| anyhow::anyhow!("Could not aquire feed lock"))?
            .len()
            - 1;
        if self
            .feed
            .write()
            .map_err(|_| anyhow::anyhow!("Could not aquire feed lock"))?
            .has(feed_length)
        {
            // Eagerly send the length of the feed to the otherside
            // TODO: only send this if the remote is not wanting a region
            // where this is contained in
            let mut have = proto::Have::new();
            have.set_start(feed_length as u64);
            client.have(channel, &have).await?;
        }
        let rle = self
            .feed
            .read()
            .map_err(|_| anyhow::anyhow!("Could not aquire feed lock"))?
            .bitfield()
            .compress(message.get_start() as usize, message.get_length() as usize)?;
        let mut have = proto::Have::new();
        have.set_start(message.get_start());
        have.set_length(message.get_length());
        have.set_bitfield(rle);
        client.have(channel, &have).await?;
        Ok(())
    }

    // https://github.com/mafintosh/hypercore/blob/84990baa477491478f79b968123d2233eebeba76/lib/replicate.js#L295-L343
    async fn on_have(
        &mut self,
        client: &mut Client,
        channel: u64,
        message: &proto::Have,
    ) -> Result<(), Self::Err> {
        // TODO implement setting the length and request data on metadata
        if message.has_bitfield() {
            let buf = bitfield_rle::decode(message.get_bitfield())
                .map_err(|e| anyhow::anyhow!(e))
                .context("could not decode bitfield")?;
            let bits = buf.len() * 8;
            // TODO
            // Compare bitfields
            // remoteAndNotLocal(this.feed.bitfield, buf, this.remoteBitfield.littleEndian, have.start)
            // TODO fill
            // self.remoteBitfield.fill(buf, message.get_start());
            if bits > self.remote_length {
                // TODO last
                // self.remoteLength = self.remoteBitfield.last() + 1;
            }
        } else {
            let mut start = message.get_start() as usize;
            let len = if message.has_length() {
                message.get_length()
            } else {
                1
            };
            for _ in 0..len {
                start += 1;
                // TODO feed.bitfield is private
                // self.remoteBitfield
                // .set(start, !self.feed.bitfield.get(start));
            }
            if start > self.remote_length {
                self.remote_length = start
            }
        }
        // .context("invalid bitfield");

        // SEND REQUEST
        // TODO loop on length
        // Use missing data from feed
        for index in 0..=message.get_start() {
            let mut request = proto::Request::new();
            request.set_index(index);
            client.request(channel, &request).await?;
        }
        Ok(())
    }

    async fn on_data(
        &mut self,
        _client: &mut Client,
        _channel: u64,
        message: &proto::Data,
    ) -> Result<(), Self::Err> {
        let proof = hypercore::Proof {
            index: message.get_index(),
            nodes: message
                .get_nodes()
                .iter()
                .map(|node| {
                    hypercore::Node::new(
                        node.get_index(),
                        node.get_hash().to_vec(),
                        node.get_size(),
                    )
                })
                .collect(),
            signature: hypercore::Signature::from_bytes(message.get_signature()).ok(),
        };

        self.feed
            .write()
            .map_err(|_| anyhow::anyhow!("Could not aquire feed lock"))?
            .put(
                message.get_index(),
                if message.has_value() {
                    Some(message.get_value())
                } else {
                    None
                },
                proof,
            )
            .context("could not write data to feed")?;
        Ok(())
    }
}
