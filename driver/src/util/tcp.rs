use anyhow::anyhow;
use embedded_io::{
    ErrorType,
    Read,
    Write,
};
use vtk_wsk::{
    sys::{
        IPPROTO_IPPROTO_TCP,
        PSOCKADDR,
        SOCKADDR_INET,
        SOCK_STREAM,
    },
    SocketAddrInetEx,
    WskBuffer,
    WskConnectionSocket,
    WskError,
    WskInstance,
};

pub struct TcpConnection {
    socket: WskConnectionSocket,
}

#[allow(unused)]
impl TcpConnection {
    pub fn connect(wsk: &WskInstance, target: &SOCKADDR_INET) -> anyhow::Result<Self> {
        let mut socket = wsk
            .create_connection_socket(
                target.si_family(),
                SOCK_STREAM as u16,
                IPPROTO_IPPROTO_TCP as u32,
            )
            .map_err(|err| anyhow!("socket: {:#}", err))?;

        let mut local_address: SOCKADDR_INET = unsafe { core::mem::zeroed() };
        local_address.si_family = target.si_family();
        socket
            .bind(local_address.as_sockaddr_mut())
            .map_err(|err| anyhow!("bind: {:#}", err))?;

        socket
            .connect(target as *const _ as PSOCKADDR)
            .map_err(|err| anyhow!("connect: {:#}", err))?;

        Ok(Self { socket })
    }
}

impl ErrorType for TcpConnection {
    type Error = WskError;
}

impl Read for TcpConnection {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let mut wsk_buffer = WskBuffer::create(buf)?;
        self.socket.receive(&mut wsk_buffer, 0)
    }
}

impl Write for TcpConnection {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let mut wsk_buffer = WskBuffer::create_ro(buf)?;

        self.socket
            .send(&mut wsk_buffer, 0)
            .inspect_err(|err| log::trace!("write error: {:#}", err))
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
