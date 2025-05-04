//! Reliable packets for point-to-point half-duplex bytestream with master and slave.
//!
//! Packets may be dropped or re-transmitted, but will never be corrupt.
//! The flags in this protocol enable fast retransmission if a package was not received correctly.

use bitfield_struct::bitfield;
use cobs::DestBufTooSmallError;
use embedded_io_async::{Read, Write};
use endian_num::le16;

const CRC_ALG: crc::Algorithm<u16> = crc::CRC_16_USB;
const CRC: crc::Crc<u16> = crc::Crc::<u16>::new(&CRC_ALG);

const HEADER_LEN: usize = 2;
const CRC_LEN: usize = 2;

const EMPTY_HEADER: Header = Header::new()
    .with_ack(false)
    .with_allow_data(true)
    .with_has_data(false)
    .with_len(0);
const EMPTY_HEADER_BYTES: [u8; HEADER_LEN] = EMPTY_HEADER.to_bytes();
const EMPTY_HEADER_CRC: u16 = CRC.checksum(&EMPTY_HEADER_BYTES);
const EMPTY_HEADER_CRC_BYTES: [u8; CRC_LEN] = EMPTY_HEADER_CRC.to_le_bytes();

const MARKER: u8 = 0x00;

struct Transceiver<T: Read + Write, const MTU: usize> {
    inner: T,
}

#[allow(async_fn_in_trait)]
pub trait PacketInterface {
    type Error;

    async fn transfer(
        &mut self,
        rx_packet: &mut [u8],
        tx_packet: Option<&[u8]>,
    ) -> Result<Option<usize>, Self::Error>;
}

#[bitfield(u16, repr = le16, from = le16::from_ne, into = le16::to_ne)]
pub struct Header {
    ack: bool,
    allow_data: bool,
    has_data: bool,
    #[bits(13)]
    len: u16,
}

impl Header {
    pub const fn to_bytes(&self) -> [u8; 2] {
        self.into_bits().to_le_bytes()
    }
}

pub enum Error<E> {
    /// Underlying buffer to store packet in is too small.
    BufferTooSmall,
    /// Missing marker in stream.
    MissingMarker,
    /// Cobs encoding is malformed.
    CobsEncoding,
    /// The header + body + checksum structure is malformed.
    Malformed,
    /// Checksum of package is incorrect.
    Checksum,
    /// The internal IO mechanism returned an error.
    Inner(E),
}

impl<E> From<DestBufTooSmallError> for Error<E> {
    fn from(_value: DestBufTooSmallError) -> Self {
        Error::BufferTooSmall
    }
}

impl<E> From<cobs::DecodeError> for Error<E> {
    fn from(_value: cobs::DecodeError) -> Self {
        Error::CobsEncoding
    }
}

impl<T: Read + Write, const MTU: usize> Transceiver<T, MTU> {
    async fn send(&mut self, header: Header, data: &[u8]) -> Result<(), Error<T::Error>> {
        let header = header.to_bytes();
        let mut digest = CRC.digest();
        digest.update(&header);
        digest.update(data);
        let crc = digest.finalize();

        let mut buf = [0u8; MTU];
        let mut encoder = cobs::CobsEncoder::new(&mut buf);
        encoder.push(&header)?;
        encoder.push(data)?;
        encoder.push(&crc.to_le_bytes())?;
        let size = encoder.finalize();

        self.inner
            .write_all(&buf[0..size])
            .await
            .map_err(Error::Inner)?;

        Ok(())
    }

    async fn send_empty_nack_request(&mut self) -> Result<(), Error<T::Error>> {
        let mut buf = [0u8; cobs::max_encoding_length(EMPTY_HEADER_BYTES.len() + 2)];
        let mut encoder = cobs::CobsEncoder::new(&mut buf);
        encoder.push(&EMPTY_HEADER_BYTES)?;
        encoder.push(&EMPTY_HEADER_CRC_BYTES)?;
        let size = encoder.finalize();

        self.inner
            .write_all(&buf[0..size])
            .await
            .map_err(Error::Inner)?;

        Ok(())
    }

    async fn receive(&mut self, data: &mut [u8]) -> Result<Header, Error<T::Error>> {
        let mut rx_buf = [0u8; MTU];
        let mut total_size = 0;

        loop {
            let size = self
                .inner
                .read(&mut rx_buf[total_size..])
                .await
                .map_err(Error::Inner)?;
            total_size += size;

            // Read should always deliver at least one byte.
            assert!(total_size > 0);

            if rx_buf[0] != MARKER {
                return Err(Error::MissingMarker);
            }

            if total_size > 1 && rx_buf[total_size - 1] == MARKER {
                // Done!
                break;
            }

            if total_size == rx_buf.len() {
                return Err(Error::BufferTooSmall);
            }
        }

        // Content without two markers at beginning and end.
        let rx_content = &mut rx_buf[1..total_size - 1];
        let rx_size = cobs::decode_in_place(rx_content)?;
        let rx_content = &rx_content[0..rx_size];

        if rx_content.len() < HEADER_LEN + CRC_LEN {
            return Err(Error::Malformed);
        }

        let (rx_header_body, rx_checksum) = rx_content.split_at(rx_content.len() - CRC_LEN);

        // Check checksum
        if CRC.checksum(rx_header_body).to_le_bytes() != rx_checksum {
            return Err(Error::Checksum);
        }

        let (rx_header, rx_body) = rx_header_body.split_at(HEADER_LEN);
        let rx_header =
            Header::from_bits(le16::from_le_bytes(defmt::unwrap!(rx_header.try_into())));

        if rx_header.len() as usize != rx_body.len() {
            return Err(Error::Malformed);
        }

        if rx_body.len() < data.len() {
            return Err(Error::BufferTooSmall);
        }

        data[0..rx_body.len()].copy_from_slice(rx_body);

        Ok(rx_header)
    }
}

pub struct Master<T: Read + Write, const MTU: usize> {
    inner: Transceiver<T, MTU>,
}

pub struct Slave<T: Read + Write, const MTU: usize> {
    inner: Transceiver<T, MTU>,
}

impl<T: Read + Write, const MTU: usize> Master<T, MTU> {
    pub const fn new(inner: T) -> Self {
        Self {
            inner: Transceiver { inner },
        }
    }
}

impl<T: Read + Write, const MTU: usize> Slave<T, MTU> {
    pub const fn new(inner: T) -> Self {
        Self {
            inner: Transceiver { inner },
        }
    }
}

impl<T: Read + Write, const MTU: usize> PacketInterface for Master<T, MTU> {
    type Error = Error<T::Error>;

    async fn transfer(
        &mut self,
        rx_packet: &mut [u8],
        tx_packet: Option<&[u8]>,
    ) -> Result<Option<usize>, Self::Error> {
        loop {
            if let Some(tx_packet) = tx_packet {
                self.inner
                    .send(
                        Header::new()
                            .with_ack(false)
                            .with_has_data(true)
                            .with_allow_data(true)
                            .with_len(tx_packet.len() as u16),
                        tx_packet,
                    )
                    .await?;
            } else {
                self.inner.send_empty_nack_request().await?;
            }

            match self.inner.receive(rx_packet).await {
                Ok(header) => {
                    if !header.ack() {
                        continue; // Try again
                    }

                    if header.has_data() {
                        self.inner
                            .send(
                                Header::new()
                                    .with_ack(true)
                                    .with_has_data(false)
                                    .with_allow_data(false)
                                    .with_len(0),
                                &[],
                            )
                            .await?;

                        return Ok(Some(header.len() as usize));
                    } else {
                        return Ok(None);
                    }
                }
                // Transmission related issues
                Err(Error::Checksum) | Err(Error::CobsEncoding) | Err(Error::MissingMarker) => {
                    // TODO mark statistics
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl<T: Read + Write, const MTU: usize> PacketInterface for Slave<T, MTU> {
    type Error = Error<T::Error>;

    async fn transfer(
        &mut self,
        rx_packet: &mut [u8],
        tx_packet: Option<&[u8]>,
    ) -> Result<Option<usize>, Self::Error> {
        let mut result = None;

        loop {
            match self.inner.receive(rx_packet).await {
                Ok(header) => {
                    if header.ack() {
                        return Ok(result);
                    }

                    if !header.allow_data() {
                        self.inner.send_empty_nack_request().await?;
                        continue;
                    }

                    // Send our data with an acknowledgement.
                    if let Some(tx_packet) = tx_packet {
                        self.inner
                            .send(
                                Header::new()
                                    .with_ack(true)
                                    .with_has_data(true)
                                    .with_len(tx_packet.len() as u16),
                                tx_packet,
                            )
                            .await?;

                        if header.has_data() {
                            result = Some(header.len() as usize);
                            continue; // Receive ACK first.
                        } else {
                            return Ok(None);
                        }
                    } else {
                        self.inner
                            .send(
                                Header::new()
                                    .with_ack(true)
                                    .with_has_data(false)
                                    .with_allow_data(true),
                                &[],
                            )
                            .await?;
                        return Ok(None);
                    }
                }
                // Transmission related issues
                Err(Error::Checksum) | Err(Error::CobsEncoding) | Err(Error::MissingMarker) => {
                    // TODO mark statistics

                    // Send a NACK, request retransmission.
                    self.inner.send_empty_nack_request().await?;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }
}
