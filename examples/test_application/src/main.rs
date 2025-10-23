extern crate pretty_env_logger;

use std::convert::Infallible;

use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    pipe::{Pipe, Reader, Writer},
};
use tinynet::ptp_packets::{Master, PacketInterface, Slave};

const PIPE_LENGTH: usize = 256;

struct MockHalfDuplexBus {
    master: Pipe<NoopRawMutex, PIPE_LENGTH>,
    slave: Pipe<NoopRawMutex, PIPE_LENGTH>,
}

struct MockHalfDuplexSide<'a> {
    name: String,
    pub rx: Reader<'a, NoopRawMutex, PIPE_LENGTH>,
    pub tx: Writer<'a, NoopRawMutex, PIPE_LENGTH>,
}

impl embedded_io_async::ErrorType for MockHalfDuplexSide<'_> {
    type Error = Infallible;
}

impl embedded_io_async::Read for MockHalfDuplexSide<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        log::trace!("{} << start", self.name);
        let result = self.rx.read(buf).await;
        log::trace!("{} << {:0x?}", self.name, &buf[0..result]);
        Ok(result)
    }
}

impl embedded_io_async::Write for MockHalfDuplexSide<'_> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let result = self.tx.write(buf).await;
        log::trace!("{} >> {:0x?}", self.name, &buf[0..result]);

        Ok(result)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(()) // No-op
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
            name: "Master".to_owned(),
            rx: master_rx,
            tx: master_tx,
        };
        let slave = MockHalfDuplexSide {
            name: "Slave".to_owned(),
            rx: slave_rx,
            tx: slave_tx,
        };
        (master, slave)
    }
}

#[async_std::main]
async fn main() {
    pretty_env_logger::init();

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

        for i in 0..5 {
            // Master RX empty
            log::debug!("Master {}", i);
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

        for i in 0..5 {
            // Slave RX empty
            log::debug!("Slave {}", i);
            let result = slave.transfer(&mut buf, None).await.unwrap();
            assert!(result.is_none());
        }
    };

    embassy_futures::join::join(master_fut, slave_fut).await;
}
