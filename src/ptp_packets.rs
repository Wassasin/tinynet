//! Reliable packets for point-to-point half-duplex bytestream with master and slave.
//!
//! Packets may be dropped or re-transmitted, but will never be corrupt.
//! The flags in this protocol enable fast retransmission if a package was not received correctly.

use bitfield_struct::bitfield;
use cobs::DestBufTooSmallError;
use embedded_io_async::{Read, Write};
use endian_num::le16;

use crate::{buf::Buf, unwrap};

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
    rx_buf: Buf<MTU>,
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

#[derive(Debug)]
pub enum Error<E> {
    /// Underlying buffer to store packet in is too small.
    BufferTooSmall,
    /// Cobs encoding is malformed.
    CobsEncoding,
    /// The header + body + checksum + marker structure is malformed.
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

impl<E> From<crate::buf::OverflowError> for Error<E> {
    fn from(_value: crate::buf::OverflowError) -> Self {
        Error::BufferTooSmall
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
        let size = {
            let mut encoder = cobs::CobsEncoder::new(&mut buf);
            encoder.push(&header)?;
            encoder.push(data)?;
            encoder.push(&crc.to_le_bytes())?;
            encoder.finalize()
        };
        buf[1 + size] = MARKER;

        self.inner
            .write_all(&buf[0..size + 1])
            .await
            .map_err(Error::Inner)?;

        Ok(())
    }

    async fn send_empty_nack_request(&mut self) -> Result<(), Error<T::Error>> {
        self.send(
            Header::new()
                .with_ack(false)
                .with_allow_data(true)
                .with_has_data(false)
                .with_len(0),
            &[],
        )
        .await
    }

    fn decode_frame(source: &mut [u8], destination: &mut [u8]) -> Result<Header, Error<T::Error>> {
        let size = cobs::decode_in_place(source)?;
        let source = &source[0..size];

        if source.len() < HEADER_LEN + CRC_LEN {
            return Err(Error::Malformed);
        }

        let (rx_header_body, rx_checksum) = source.split_at(source.len() - CRC_LEN);

        // Check checksum
        if CRC.checksum(rx_header_body).to_le_bytes() != rx_checksum {
            return Err(Error::Checksum);
        }

        let (rx_header, rx_body) = rx_header_body.split_at(HEADER_LEN);
        let rx_header = if let Ok(rx_header) = rx_header.try_into() {
            Header::from_bits(le16::from_le_bytes(rx_header))
        } else {
            // We just split on HEADER_LEN, so the conversion must succeed.
            unreachable!();
        };

        if rx_header.len() as usize != rx_body.len() {
            return Err(Error::Malformed);
        }

        if rx_body.len() > destination.len() {
            return Err(Error::BufferTooSmall);
        }

        destination[0..rx_body.len()].copy_from_slice(rx_body);

        Ok(rx_header)
    }

    async fn receive(&mut self, data: &mut [u8]) -> Result<Header, Error<T::Error>> {
        let mut scanned_upto = 0usize;

        // Pull from underlying interface until we receive at least a full frame.
        let frame_len = loop {
            // To start off, check if we have a full frame already in our rx buffer.
            let marker = self.rx_buf.as_slice()[scanned_upto..]
                .into_iter()
                .enumerate()
                .find(|(_i, b)| **b == MARKER);
            scanned_upto = self.rx_buf.len();

            if let Some((marker_i, _)) = marker {
                break marker_i;
            }

            if self.rx_buf.is_full() {
                // Give up on our current receive buffer, it is full of garbage.
                // TODO keep statistics
                self.rx_buf.clear();
                continue;
            }

            let size = self
                .inner
                .read(&mut self.rx_buf.free_mut_slice())
                .await
                .map_err(Error::Inner)?;

            // Note(unwrap): free_mut_slice max size corresponds to available space in rx_buf,
            // hence size can only be bigger if `inner` does not abide by the IO interface rules.
            unwrap!(self.rx_buf.mark_valid(size));
        };

        let result = Self::decode_frame(&mut self.rx_buf.as_mut_slice()[..frame_len], data);

        // Regardless of result, delete the candidate frame data.
        self.rx_buf.truncate_front(frame_len + 1);

        result
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
            inner: Transceiver {
                rx_buf: Buf::new(),
                inner,
            },
        }
    }
}

impl<T: Read + Write, const MTU: usize> Slave<T, MTU> {
    pub const fn new(inner: T) -> Self {
        Self {
            inner: Transceiver {
                rx_buf: Buf::new(),
                inner,
            },
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
                    if tx_packet.is_some() && !header.ack() {
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
                Err(Error::Checksum) | Err(Error::CobsEncoding) => {
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

                    if header.has_data() {
                        result = Some(header.len() as usize);
                    }

                    // Send our data with an acknowledgement.
                    if let Some(tx_packet) = tx_packet {
                        if !header.allow_data() {
                            self.inner.send_empty_nack_request().await?;
                            continue;
                        }

                        self.inner
                            .send(
                                Header::new()
                                    .with_ack(header.has_data())
                                    .with_has_data(true)
                                    .with_len(tx_packet.len() as u16),
                                tx_packet,
                            )
                            .await?;

                        continue; // Receive ACK first.
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
                        if header.has_data() {
                            return Ok(result);
                        } else {
                            return Ok(None);
                        }
                    }
                }
                // Transmission related issues
                Err(Error::Checksum) | Err(Error::CobsEncoding) => {
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
