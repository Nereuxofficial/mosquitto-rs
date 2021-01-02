use crate::Error;
pub(crate) use libmosquitto_sys as sys;
use std::cell::{Ref, RefCell};
use std::convert::TryInto;
use std::ffi::{CStr, CString};
use std::os::raw::{c_int, c_void};
use std::sync::Arc;
use std::sync::Once;

static INIT: Once = Once::new();

fn init_library() {
    // Note: we never call mosquitto_lib_cleanup as we can't ever
    // know when it will be safe to do so.
    INIT.call_once(|| unsafe {
        sys::mosquitto_lib_init();
    });
}

/// Represents the version of the linked mosquitto client library
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct LibraryVersion {
    /// The major version of the library
    pub major: c_int,
    /// The minor version of the library
    pub minor: c_int,
    /// The revision of the library
    pub revision: c_int,
    /// A unique number based on the major, minor and revision values
    pub version: c_int,
}

impl std::fmt::Display for LibraryVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.minor, self.major, self.revision)
    }
}

/// Returns the version information for the linked mosquitto library
pub fn lib_version() -> LibraryVersion {
    init_library();

    let mut vers = LibraryVersion {
        major: 0,
        minor: 0,
        revision: 0,
        version: 0,
    };
    unsafe {
        vers.version =
            sys::mosquitto_lib_version(&mut vers.major, &mut vers.minor, &mut vers.revision);
    }
    vers
}

pub(crate) fn cstr(s: &str) -> Result<CString, Error> {
    Ok(CString::new(s)?)
}

/// `Mosq` is the low-level mosquitto client.
/// You probably want to look at [Client](struct.Client.html) instead.
pub struct Mosq<CB = ()>
where
    CB: Callbacks,
{
    m: *mut sys::mosquitto,
    cb: Option<Arc<CallbackWrapper<CB>>>,
}

// libmosquitto is internally thread safe, so tell the rust compiler
// that the Mosq wrapper type is Sync and Send.
unsafe impl<CB: Callbacks> Sync for Mosq<CB> {}
unsafe impl<CB: Callbacks> Send for Mosq<CB> {}

impl<CB: Callbacks> Mosq<CB> {
    /// Create a new client instance with a random client id
    pub fn with_auto_id(callbacks: CB) -> Result<Self, Error> {
        init_library();
        unsafe {
            let cb = Arc::new(CallbackWrapper::new(callbacks));
            let m = sys::mosquitto_new(std::ptr::null(), true, Arc::as_ptr(&cb) as *mut _);
            if m.is_null() {
                Err(Error::Create(std::io::Error::last_os_error()))
            } else {
                Ok(Self::set_callbacks(Self { m, cb: Some(cb) }))
            }
        }
    }

    /// Create a new client instance with the specified id.
    /// If clean_session is true, instructs the broker to clean all messages
    /// and subscriptions on disconnect.  Otherwise it will preserve them.
    pub fn with_id(callbacks: CB, id: &str, clean_session: bool) -> Result<Self, Error> {
        init_library();
        unsafe {
            let cb = Arc::new(CallbackWrapper::new(callbacks));
            let m = sys::mosquitto_new(
                cstr(id)?.as_ptr(),
                clean_session,
                Arc::as_ptr(&cb) as *mut _,
            );
            if m.is_null() {
                Err(Error::Create(std::io::Error::last_os_error()))
            } else {
                Ok(Self::set_callbacks(Self { m, cb: Some(cb) }))
            }
        }
    }

    /// Configure the client with an optional username and password.
    /// The default is `None` for both.
    /// Whether you need to configure these credentials depends on the
    /// broker configuration.
    pub fn set_username_and_password(
        &self,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<(), Error> {
        let user;
        let pass;
        let username = match username {
            Some(u) => {
                user = cstr(u)?;
                user.as_ptr()
            }
            None => std::ptr::null(),
        };

        let password = match password {
            Some(p) => {
                pass = cstr(p)?;
                pass.as_ptr()
            }
            None => std::ptr::null(),
        };

        let err = unsafe { sys::mosquitto_username_pw_set(self.m, username, password) };

        Error::result(err, ())
    }

    /// Connect to the broker on the specified host and port.
    /// port is typically 1883 for mqtt, but it may be different
    /// in your environment.
    ///
    /// `keep_alive_interval` specifies the interval at which
    /// keepalive requests are sent.  mosquitto has a minimum value
    /// of 5 for this and will generate an error if you use a smaller
    /// value.
    ///
    /// `bind_address` can be used to specify the outgoing interface
    /// for the connection.
    pub fn connect(
        &self,
        host: &str,
        port: c_int,
        keep_alive_interval: std::time::Duration,
        bind_address: Option<&str>,
    ) -> Result<(), Error> {
        let host = cstr(host)?;
        let ba;
        let bind_address = match bind_address {
            Some(b) => {
                ba = cstr(b)?;
                ba.as_ptr()
            }
            None => std::ptr::null(),
        };
        let err = unsafe {
            sys::mosquitto_connect_bind(
                self.m,
                host.as_ptr(),
                port,
                keep_alive_interval
                    .as_secs()
                    .try_into()
                    .map_err(|_| Error::Mosq(sys::mosq_err_t::MOSQ_ERR_INVAL))?,
                bind_address,
            )
        };
        Error::result(err, ())
    }

    /// Connect to the broker on the specified host and port,
    /// but don't block for the connection portion.
    /// (Note that name resolution may still block!).
    ///
    /// The connection will be completed later by running the message loop
    /// using either `loop_until_explicitly_disconnected` or
    /// `start_loop_thread`.
    ///
    /// port is typically 1883 for mqtt, but it may be different
    /// in your environment.
    ///
    /// `keep_alive_interval` specifies the interval at which
    /// keepalive requests are sent.  mosquitto has a minimum value
    /// of 5 for this and will generate an error if you use a smaller
    /// value.
    ///
    /// `bind_address` can be used to specify the outgoing interface
    /// for the connection.
    pub fn connect_non_blocking(
        &self,
        host: &str,
        port: c_int,
        keep_alive_interval: std::time::Duration,
        bind_address: Option<&str>,
    ) -> Result<(), Error> {
        let host = cstr(host)?;
        let ba;
        let bind_address = match bind_address {
            Some(b) => {
                ba = cstr(b)?;
                ba.as_ptr()
            }
            None => std::ptr::null(),
        };
        let err = unsafe {
            sys::mosquitto_connect_bind_async(
                self.m,
                host.as_ptr(),
                port,
                keep_alive_interval
                    .as_secs()
                    .try_into()
                    .map_err(|_| Error::Mosq(sys::mosq_err_t::MOSQ_ERR_INVAL))?,
                bind_address,
            )
        };
        Error::result(err, ())
    }

    /// Reconnect a disconnected client using the same parameters
    /// as were originally used to connect it.
    pub fn reconnect(&self) -> Result<(), Error> {
        Error::result(unsafe { sys::mosquitto_reconnect(self.m) }, ())
    }

    /// Disconnect the client.
    /// This will cause the message loop to terminate.
    pub fn disconnect(&self) -> Result<(), Error> {
        Error::result(unsafe { sys::mosquitto_disconnect(self.m) }, ())
    }

    /// Publish a message to the specified topic.
    ///
    /// The payload size can be 0-283, 435 or 455 bytes; other values
    /// will generate an error result.
    ///
    /// `retain` will set the message to be retained by the broker,
    /// and delivered to new subscribers.
    ///
    /// Returns the assigned MessageId value for the publish.
    /// The publish may not complete immediately.
    /// Your `Callbacks::on_publish` handler will be called
    /// when it completes.
    pub fn publish(
        &self,
        topic: &str,
        payload: &[u8],
        qos: QoS,
        retain: bool,
    ) -> Result<MessageId, Error> {
        let mut mid = 0;
        let err = unsafe {
            sys::mosquitto_publish(
                self.m,
                &mut mid,
                cstr(topic)?.as_ptr(),
                payload
                    .len()
                    .try_into()
                    .map_err(|_| Error::Mosq(sys::mosq_err_t::MOSQ_ERR_PAYLOAD_SIZE))?,
                payload.as_ptr() as *const _,
                qos as c_int,
                retain,
            )
        };
        Error::result(err, mid)
    }

    /// Establish a subscription for topics that match `pattern`.
    ///
    /// Your `Callbacks::on_message` handler will be called as messages
    /// matching your subscription arrive.
    ///
    /// Returns the MessageId of the subscription request; the subscriptions
    /// won't be active until the broker has processed the request.
    /// You can use an `on_subscribe` handler to determine when that is ready.
    pub fn subscribe(&self, pattern: &str, qos: QoS) -> Result<MessageId, Error> {
        let mut mid = 0;
        let err = unsafe {
            sys::mosquitto_subscribe(self.m, &mut mid, cstr(pattern)?.as_ptr(), qos as _)
        };
        Error::result(err, mid)
    }

    fn set_callbacks(self) -> Self {
        unsafe {
            sys::mosquitto_connect_callback_set(self.m, Some(CallbackWrapper::<CB>::connect));
            sys::mosquitto_disconnect_callback_set(self.m, Some(CallbackWrapper::<CB>::disconnect));
            sys::mosquitto_publish_callback_set(self.m, Some(CallbackWrapper::<CB>::publish));
            sys::mosquitto_subscribe_callback_set(self.m, Some(CallbackWrapper::<CB>::subscribe));
            sys::mosquitto_message_callback_set(self.m, Some(CallbackWrapper::<CB>::message));
        }
        self
    }

    /// Returns a reference to the callbacks previously registered
    /// during construction.
    pub fn get_callbacks(&self) -> Ref<CB> {
        self.cb
            .as_ref()
            .expect("get_callbacks not to be called on a transient Mosq")
            .cb
            .borrow()
    }

    /// Runs the message loop for the client.
    /// This method will not return until the client is explicitly
    /// disconnected via the `disconnect` method.
    ///
    /// `timeout` specifies the internal sleep duration between
    /// iterations.
    pub fn loop_until_explicitly_disconnected(
        &self,
        timeout: std::time::Duration,
    ) -> Result<(), Error> {
        unsafe {
            let max_packets = 1;
            Error::result(
                sys::mosquitto_loop_forever(
                    self.m,
                    timeout
                        .as_millis()
                        .try_into()
                        .map_err(|_| Error::Mosq(sys::mosq_err_t::MOSQ_ERR_INVAL))?,
                    max_packets,
                ),
                (),
            )
        }
    }

    /// Starts a new thread to run the message loop for the client.
    /// The thread will run until the client is disconnected,
    /// or until `stop_loop_thread` is called.
    pub fn start_loop_thread(&self) -> Result<(), Error> {
        unsafe { Error::result(sys::mosquitto_loop_start(self.m), ()) }
    }

    /// Stops the message loop thread started via `start_loop_thread`
    pub fn stop_loop_thread(&self, force_cancel: bool) -> Result<(), Error> {
        unsafe { Error::result(sys::mosquitto_loop_stop(self.m, force_cancel), ()) }
    }
}

struct CallbackWrapper<T: Callbacks> {
    cb: RefCell<T>,
}

fn with_transient_client<F: FnOnce(&mut Mosq)>(m: *mut sys::mosquitto, func: F) {
    let mut client = Mosq { m, cb: None };
    func(&mut client);
    std::mem::forget(client);
}

impl<T: Callbacks> CallbackWrapper<T> {
    fn new(cb: T) -> Self {
        Self {
            cb: RefCell::new(cb),
        }
    }

    unsafe fn resolve_self<'a>(cb: *mut c_void) -> &'a Self {
        &*(cb as *const Self)
    }

    unsafe extern "C" fn connect(m: *mut sys::mosquitto, cb: *mut c_void, rc: c_int) {
        let cb = Self::resolve_self(cb);
        with_transient_client(m, |client| {
            cb.cb.borrow().on_connect(client, rc);
        });
    }

    unsafe extern "C" fn disconnect(m: *mut sys::mosquitto, cb: *mut c_void, rc: c_int) {
        let cb = Self::resolve_self(cb);
        with_transient_client(m, |client| {
            cb.cb.borrow().on_disconnect(client, rc);
        });
    }

    unsafe extern "C" fn publish(m: *mut sys::mosquitto, cb: *mut c_void, mid: MessageId) {
        let cb = Self::resolve_self(cb);
        with_transient_client(m, |client| {
            cb.cb.borrow().on_publish(client, mid);
        });
    }

    unsafe extern "C" fn subscribe(
        m: *mut sys::mosquitto,
        cb: *mut c_void,
        mid: MessageId,
        qos_count: c_int,
        granted_qos: *const c_int,
    ) {
        let cb = Self::resolve_self(cb);
        with_transient_client(m, |client| {
            let granted_qos = std::slice::from_raw_parts(granted_qos, qos_count as usize);
            let granted_qos: Vec<QoS> = granted_qos.iter().map(QoS::from_int).collect();
            cb.cb.borrow().on_subscribe(client, mid, &granted_qos);
        });
    }

    unsafe extern "C" fn message(
        m: *mut sys::mosquitto,
        cb: *mut c_void,
        msg: *const sys::mosquitto_message,
    ) {
        let cb = Self::resolve_self(cb);
        with_transient_client(m, |client| {
            let msg = &*msg;
            let topic = CStr::from_ptr(msg.topic);
            let topic = topic.to_string_lossy().to_string();
            cb.cb.borrow().on_message(
                client,
                msg.mid,
                topic,
                std::slice::from_raw_parts(msg.payload as *const u8, msg.payloadlen as usize),
                QoS::from_int(&msg.qos),
                msg.retain,
            );
        });
    }
}

/// Represents an individual message identifier.
/// This is used in this client to determine when a message
/// has been sent.
pub type MessageId = c_int;

/// Defines handlers that can be used to determine when various
/// functions have completed.
pub trait Callbacks {
    /// called when the connection has been acknowledged by the broker.
    /// `reason` holds the connection return code; the value depends on the
    /// version of the MQTT protocol in use:
    /// For MQTT v5.0, look at section 3.2.2.2 Connect Reason code: <https://docs.oasis-open.org/mqtt/mqtt/v5.0/os/mqtt-v5.0-os.html>
    /// For MQTT v3.1.1, look at section 3.2.2.3 Connect Return code: <http://docs.oasis-open.org/mqtt/mqtt/v3.1.1/mqtt-v3.1.1.html>
    fn on_connect(&self, _client: &mut Mosq, _reason: c_int) {}

    /// Called when the broker has received the DISCONNECT command
    fn on_disconnect(&self, _client: &mut Mosq, _reason: c_int) {}

    /// Called when the message identifier by `mid` has been sent
    /// to the broker successfully.
    fn on_publish(&self, _client: &mut Mosq, _mid: MessageId) {}

    /// Called when the broker responds to a subscription request.
    fn on_subscribe(&self, _client: &mut Mosq, _mid: MessageId, _granted_qos: &[QoS]) {}

    /// Called when a message matching a subscription is received
    /// from the broker
    fn on_message(
        &self,
        _client: &mut Mosq,
        _mid: MessageId,
        _topic: String,
        _payload: &[u8],
        _qos: QoS,
        _retain: bool,
    ) {
    }
}

impl Callbacks for () {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QoS {
    /// This is the simplest, lowest-overhead method of sending a message. The client simply
    /// publishes the message, and there is no acknowledgement by the broker.
    AtMostOnce = 0,
    /// This method guarantees that the message will be transferred successfully to the broker.
    /// The broker sends an acknowledgement back to the sender, but in the event that that the
    /// acknowledgement is lost the sender won't realise the message has got through, so will send
    /// the message again. The client will re-send until it gets the broker's acknowledgement.
    /// This means that sending is guaranteed, although the message may reach the broker more than
    /// once.
    AtLeastOnce = 1,
    /// This is the highest level of service, in which there is a sequence of four messages between
    /// the sender and the receiver, a kind of handshake to confirm that the main message has been
    /// sent and that the acknowledgement has been received.  When the handshake has been
    /// completed, both sender and receiver are sure that the message was sent exactly once.
    ExactlyOnce = 2,
}

impl Default for QoS {
    fn default() -> QoS {
        QoS::AtMostOnce
    }
}

impl QoS {
    fn from_int(i: &c_int) -> QoS {
        match i {
            0 => Self::AtMostOnce,
            1 => Self::AtLeastOnce,
            2 => Self::ExactlyOnce,
            _ => Self::ExactlyOnce,
        }
    }
}

impl<CB: Callbacks> Drop for Mosq<CB> {
    fn drop(&mut self) {
        unsafe {
            sys::mosquitto_destroy(self.m);
        }
    }
}
