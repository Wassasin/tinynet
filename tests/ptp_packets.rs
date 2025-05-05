use std::convert::Infallible;

use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    pipe::{Pipe, Reader, Writer},
};
use embedded_io_async::{Read, Write};
use mac_rf::ptp_packets::{Master, PacketInterface, Slave};

const PIPE_LENGTH: usize = 128;

struct MockHalfDuplexBus {
    master: Pipe<NoopRawMutex, PIPE_LENGTH>,
    slave: Pipe<NoopRawMutex, PIPE_LENGTH>,
}

struct MockHalfDuplexSide<'a> {
    pub rx: Reader<'a, NoopRawMutex, PIPE_LENGTH>,
    pub tx: Writer<'a, NoopRawMutex, PIPE_LENGTH>,
}

impl embedded_io_async::ErrorType for MockHalfDuplexSide<'_> {
    type Error = Infallible;
}

impl embedded_io_async::Read for MockHalfDuplexSide<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(self.rx.read(buf).await)
    }
}

impl embedded_io_async::Write for MockHalfDuplexSide<'_> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        Ok(self.tx.write(buf).await)
    }
}

impl MockHalfDuplexBus {
    pub fn new() -> Self {
        Self {
            master: Pipe::new(),
            slave: Pipe::new(),
        }
    }

    pub fn split(&mut self) -> (MockHalfDuplexSide<'_>, MockHalfDuplexSide<'_>) {
        let (master_rx, slave_tx) = self.master.split();
        let (slave_rx, master_tx) = self.slave.split();

        let master = MockHalfDuplexSide {
            rx: master_rx,
            tx: master_tx,
        };
        let slave = MockHalfDuplexSide {
            rx: slave_rx,
            tx: slave_tx,
        };
        (master, slave)
    }
}

#[async_std::test]
async fn mock() {
    let mut bus = MockHalfDuplexBus::new();
    let (mut master, mut slave) = bus.split();

    let reference = core::array::from_fn::<u8, 128, _>(|i| i as u8);
    master.write_all(&reference).await.unwrap();

    let mut buf = [0u8; 256];
    assert_eq!(slave.read(&mut buf).await, Ok(128));
    assert_eq!(buf[0..reference.len()], reference);

    let reference = core::array::from_fn::<u8, 128, _>(|i| i as u8 + 1);
    slave.write_all(&reference).await.unwrap();

    let mut buf = [0u8; 256];
    assert_eq!(master.read(&mut buf).await, Ok(128));
    assert_eq!(buf[0..reference.len()], reference);
}

#[async_std::test]
async fn rx_tx() {
    let mut bus = MockHalfDuplexBus::new();
    let (master, slave) = bus.split();

    let mut master: Master<_, 256> = Master::new(master);
    let mut slave: Slave<_, 256> = Slave::new(slave);

    let pkt = &[1, 2, 3, 4];

    let master_fut = async {
        let mut buf = [0u8; 64];

        // Master RX
        let size = master.transfer(&mut buf, None).await.unwrap().unwrap();
        assert_eq!(&buf[0..size], pkt);

        // Master TX
        let result = master.transfer(&mut buf, Some(pkt)).await.unwrap();
        assert!(result.is_none());

        // Master TX+RX
        let size = master.transfer(&mut buf, Some(pkt)).await.unwrap().unwrap();
        assert_eq!(&buf[0..size], pkt);

        for _ in 0..5 {
            // Master RX empty
            let result = master.transfer(&mut buf, None).await.unwrap();
            assert!(result.is_none());
        }
    };
    let slave_fut = async {
        let mut buf = [0u8; 64];

        // Slave TX
        let result = slave.transfer(&mut buf, Some(pkt)).await.unwrap();
        assert!(result.is_none());

        // Slave RX
        let size = slave.transfer(&mut buf, None).await.unwrap().unwrap();
        assert_eq!(&buf[0..size], pkt);

        // Slave TX+RX
        let size = slave.transfer(&mut buf, Some(pkt)).await.unwrap().unwrap();
        assert_eq!(&buf[0..size], pkt);

        for _ in 0..5 {
            // Slave RX empty
            let result = slave.transfer(&mut buf, None).await.unwrap();
            assert!(result.is_none());
        }
    };

    embassy_futures::join::join(master_fut, slave_fut).await;
}
