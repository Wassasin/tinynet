//! Reliable packets for point-to-point half-duplex bytestream with master and slave.
//!
//! Packets may be dropped or re-transmitted, but will never be corrupt.
//! The flags in this protocol enable fast retransmission if a package was not received correctly.

use bitfields::bitfield;
use cobs::DestBufTooSmallError;
use embedded_io_async::{Read, Write};

use crate::buf::Buf;

const CRC_ALG: crc::Algorithm<u16> = crc::CRC_16_USB;
const CRC: crc::Crc<u16> = crc::Crc::<u16>::new(&CRC_ALG);

const HEADER_LEN: usize = Header::new().to_bytes().len();
const CRC_LEN: usize = CRC.checksum(&[0u8; 2]).to_le_bytes().len();

/// Cobs sentinel value put at end of a frame.
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

#[bitfield(u16)]
struct Header {
    ack: bool,
    allow_data: bool,
    has_data: bool,
    #[bits(13)]
    len: u16,
}

impl Header {
    pub const fn to_bytes(self) -> [u8; 2] {
        self.into_bits().to_le_bytes()
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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

        if size + 1 >= MTU {
            return Err(Error::BufferTooSmall);
        }
        buf[size + 1] = MARKER;

        debug!("Transceiver sending {:?}", &buf[0..size + 1]);

        self.inner
            .write_all(&buf[0..size + 1])
            .await
            .map_err(Error::Inner)?;

        Ok(())
    }

    async fn send_empty_nack_request(&mut self) -> Result<(), Error<T::Error>> {
        self.send(
            HeaderBuilder::new()
                .with_ack(false)
                .with_allow_data(true)
                .with_has_data(false)
                .with_len(0)
                .build(),
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
            Header::from_bits(u16::from_le_bytes(rx_header))
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
            if self.rx_buf.is_full() {
                // Give up on our current receive buffer, it is full of garbage.
                // TODO keep statistics
                warn!("Purged buffer of garbage");
                self.rx_buf.clear();
                scanned_upto = 0;
                continue;
            }

            // To start off, check if we have a full frame already in our rx buffer.
            let marker = self.rx_buf.as_slice()[scanned_upto..]
                .iter()
                .enumerate()
                .find(|(_i, b)| **b == MARKER);

            if let Some((marker_i, _)) = marker {
                trace!("Found marker at {}", marker_i);
                break scanned_upto + marker_i;
            }

            scanned_upto = self.rx_buf.len();

            let size = self
                .inner
                .read(self.rx_buf.free_mut_slice())
                .await
                .map_err(Error::Inner)?;

            trace!("Got {:?}", &self.rx_buf.free_mut_slice()[0..size]);

            // Note(unwrap): free_mut_slice max size corresponds to available space in rx_buf,
            // hence size can only be bigger if `inner` does not abide by the IO interface rules.
            unwrap!(self.rx_buf.mark_valid(size));
        };

        let result = Self::decode_frame(&mut self.rx_buf.as_mut_slice()[..frame_len], data);

        // Regardless of result, delete the candidate frame data.
        self.rx_buf.truncate_front(frame_len + 1);

        trace!("Buffer at {}", self.rx_buf.len());

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
                        HeaderBuilder::new()
                            .with_ack(false)
                            .with_has_data(true)
                            .with_allow_data(true)
                            .with_len(tx_packet.len() as u16)
                            .build(),
                        tx_packet,
                    )
                    .await?;
            } else {
                self.inner.send_empty_nack_request().await?;
            }

            match self.inner.receive(rx_packet).await {
                Ok(header) => {
                    if tx_packet.is_some() && !header.ack() {
                        warn!("Sent packet failure");
                        continue; // Try again
                    }

                    if header.has_data() {
                        self.inner
                            .send(
                                HeaderBuilder::new()
                                    .with_ack(true)
                                    .with_has_data(false)
                                    .with_allow_data(false)
                                    .with_len(0)
                                    .build(),
                                &[],
                            )
                            .await?;
                        trace!("Master received slave data");
                        return Ok(Some(header.len() as usize));
                    } else {
                        trace!("Master received slave empty");
                        return Ok(None);
                    }
                }
                // Transmission related issues
                Err(e @ Error::Checksum)
                | Err(e @ Error::Malformed)
                | Err(e @ Error::CobsEncoding) => {
                    // TODO fix this distinction
                    #[cfg(feature = "defmt")]
                    error!("Master error {}", defmt::Debug2Format(&e));
                    #[cfg(not(feature = "defmt"))]
                    error!("Master error {:?}", e);

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
                            warn!("Slave wants to send data but master is not having it");
                            self.inner.send_empty_nack_request().await?;
                            continue;
                        }

                        self.inner
                            .send(
                                HeaderBuilder::new()
                                    .with_ack(header.has_data())
                                    .with_has_data(true)
                                    .with_len(tx_packet.len() as u16)
                                    .build(),
                                tx_packet,
                            )
                            .await?;

                        continue; // Receive ACK first.
                    } else {
                        self.inner
                            .send(
                                HeaderBuilder::new()
                                    .with_ack(true)
                                    .with_has_data(false)
                                    .with_allow_data(true)
                                    .build(),
                                &[],
                            )
                            .await?;
                        if header.has_data() {
                            return Ok(result);
                        } else {
                            trace!("Slave received master empty");
                            return Ok(None);
                        }
                    }
                }
                // Transmission related issues
                Err(e @ Error::Checksum)
                | Err(e @ Error::Malformed)
                | Err(e @ Error::CobsEncoding) => {
                    // TODO mark statistics
                    // Send a NACK, request retransmission.

                    // TODO fix this distinction
                    #[cfg(feature = "defmt")]
                    error!("Slave error {}", defmt::Debug2Format(&e));
                    #[cfg(not(feature = "defmt"))]
                    error!("Slave error {:?}", e);

                    self.inner.send_empty_nack_request().await?;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }
}
