#![cfg(not(feature = "ipc-cluster"))]

use factor::{builder::ActorBuilderConfig, prelude::*};
use futures::channel::oneshot;
use std::sync::atomic::{AtomicU16, AtomicU8, Ordering};

const SYN_SEQ_RANDOM_A: u16 = 8;
const SYN_ACK_SEQ_RANDOM_B: u16 = 5;
const PUSH_PAYLOAD: u16 = 453;

static CHECK: AtomicU8 = AtomicU8::new(0);
static ASSERT_SYN_ACK_HDR_SEQ_ACK: AtomicU16 = AtomicU16::new(0);
static ASSERT_ACK_HDR_SEQ_SYN: AtomicU16 = AtomicU16::new(0);
static ASSERT_ACK_HDR_SEQ_ACK: AtomicU16 = AtomicU16::new(0);
static ASSERT_PUSH_PAYLOAD: AtomicU16 = AtomicU16::new(0);

//==============================================================================
// PushNotification
//==============================================================================
struct PushNotification(u16);
impl Message for PushNotification {
    type Result = Option<u16>;
}

//==============================================================================
// MessageHeader
//==============================================================================
enum Mask {
    SYN,
    ACK,
}

struct MessageHeader<R: ActorReceiver> {
    bitmask: u8,
    syn_seq: u16,
    ack_seq: u16,
    source: Option<ActorWeakAddr<R>>,
}

impl<R: ActorReceiver> Message for MessageHeader<R> {
    type Result = MessageHeaderResponse<R>;
}

struct MessageHeaderResponse<R: ActorReceiver> {
    msg: MessageHeader<R>,
    tx: Option<oneshot::Sender<MessageHeader<R>>>,
}

impl<R: ActorReceiver> MessageHeaderResponse<R> {
    fn new() -> Self {
        Self {
            msg: MessageHeader::new(),
            tx: None,
        }
    }
}

impl<R: ActorReceiver> MessageHeader<R> {
    const SYN_BIT: u8 = 0b0000_0001;
    const ACK_BIT: u8 = 0b0000_0010;

    pub fn new() -> Self {
        Self {
            bitmask: 0b0000_0000,
            syn_seq: 0,
            ack_seq: 0,
            source: None,
        }
    }

    #[allow(dead_code)]
    pub fn get_mask(&self, mask: &Mask) -> bool {
        let r;
        match mask {
            Mask::SYN => r = self.bitmask & Self::SYN_BIT,
            Mask::ACK => r = self.bitmask & Self::ACK_BIT,
        }

        if r == 0 {
            false
        } else {
            true
        }
    }

    fn set_mask(&mut self, b: bool, mask: &Mask) {
        match mask {
            Mask::SYN => {
                if b {
                    self.bitmask |= Self::SYN_BIT;
                } else {
                    self.bitmask &= !Self::SYN_BIT;
                }
            }
            Mask::ACK => {
                if b {
                    self.bitmask |= Self::ACK_BIT;
                } else {
                    self.bitmask &= !Self::ACK_BIT;
                }
            }
        }
    }
}

//==============================================================================
// HandshakeClient
//==============================================================================

struct HandshakeClient {
    server: ActorAddr<HandshakeServer>,
}

impl ActorReceiver for HandshakeClient {
    type Context = BasicContext<Self>;

    fn finalize_init(&mut self, ctx: &mut Self::Context) {
        CHECK.fetch_add(1, Ordering::SeqCst);

        let fut = Self::handshake(self.server.clone(), ctx.address().unwrap().clone());
        ctx.spawn_ok(async { fut.await });
    }
}

impl HandshakeClient {
    fn new(server: ActorAddr<HandshakeServer>) -> Self {
        Self { server }
    }

    async fn handshake(server: ActorAddr<HandshakeServer>, self_addr: ActorAddr<Self>) {
        CHECK.fetch_add(1, Ordering::SeqCst);
        let mut syn_msg = MessageHeader::<Self>::new();
        syn_msg.syn_seq = SYN_SEQ_RANDOM_A;
        syn_msg.set_mask(true, &Mask::SYN);

        server
            .ask(syn_msg)
            .await
            .map(|response| {
                ASSERT_SYN_ACK_HDR_SEQ_ACK.store(response.msg.ack_seq, Ordering::SeqCst);
                // assert_eq!(response.msg.ack_seq, SYN_SEQ_RANDOM_A + 1);

                let mut ack_msg = MessageHeader::<Self>::new();
                ack_msg.syn_seq = response.msg.ack_seq;
                ack_msg.ack_seq = response.msg.syn_seq + 1;
                ack_msg.set_mask(true, &Mask::ACK);
                ack_msg.source = Some(self_addr.downgrade());

                if let Some(tx) = response.tx {
                    if !tx.is_canceled() {
                        let _ = tx.send(ack_msg);
                    }
                }
            })
            .map_err(|e| panic!("server_ask_error: {:?}", e))
            .err();
    }
}

impl MessageHandler<PushNotification> for HandshakeClient {
    type Result = MessageResponseType<<PushNotification as Message>::Result>;

    fn handle(&mut self, msg: PushNotification, _ctx: &mut Self::Context) -> Self::Result {
        ASSERT_PUSH_PAYLOAD.store(msg.0, Ordering::SeqCst);
        MessageResponseType::Result(None.into())
    }
}

//==============================================================================
// HandshakeServer
//==============================================================================

struct HandshakeServer {}
impl ActorReceiver for HandshakeServer {
    type Context = BasicContext<Self>;

    fn finalize_init(&mut self, _ctx: &mut Self::Context) {
        CHECK.fetch_add(1, Ordering::SeqCst);
    }
}

impl MessageHandler<MessageHeader<HandshakeClient>> for HandshakeServer {
    type Result = MessageResponseType<<MessageHeader<HandshakeClient> as Message>::Result>;

    fn handle(
        &mut self,
        msg: MessageHeader<HandshakeClient>,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        CHECK.fetch_add(1, Ordering::SeqCst);

        let mut syn_ack_msg = MessageHeader::new();
        syn_ack_msg.syn_seq = SYN_ACK_SEQ_RANDOM_B;
        syn_ack_msg.ack_seq = msg.syn_seq + 1;
        syn_ack_msg.set_mask(true, &Mask::SYN);
        syn_ack_msg.set_mask(true, &Mask::ACK);

        let (tx, rx) = oneshot::channel::<MessageHeader<HandshakeClient>>();
        let mut response = MessageHeaderResponse::new();
        response.msg = syn_ack_msg;
        response.tx = Some(tx);

        ctx.spawn_ok(async {
            if let Ok(msg_hdr) = rx.await {
                ASSERT_ACK_HDR_SEQ_SYN.store(msg_hdr.syn_seq, Ordering::SeqCst);
                // assert_eq!(msg_hdr.syn_seq, SYN_SEQ_RANDOM_A + 1);

                ASSERT_ACK_HDR_SEQ_ACK.store(msg_hdr.ack_seq, Ordering::SeqCst);
                // assert_eq!(msg_hdr.ack_seq, SYN_ACK_SEQ_RANDOM_B + 1);

                if let Some(weak_ref) = msg_hdr.source {
                    if let Some(client) = weak_ref.upgrade() {
                        let _ = client.tell(PushNotification(PUSH_PAYLOAD));
                    }
                }
            }
        });

        MessageResponseType::Result(ResponseResult(response))
    }
}

//==============================================================================
// TEST
//==============================================================================

#[tokio::test]
async fn test_handshake() {
    let sys = factor::init_system(Some("TestSystem".to_string()));

    let srv_spawn =
        builder::ActorBuilder::create(|_| HandshakeServer {}, &sys, ActorBuilderConfig::default());
    let server = sys.run_actor(srv_spawn.unwrap());

    let cl_spawn = builder::ActorBuilder::create(
        move |_| HandshakeClient::new(server.clone()),
        &sys,
        ActorBuilderConfig::default(),
    );
    let _client = sys.run_actor(cl_spawn.unwrap());

    std::thread::sleep(std::time::Duration::from_millis(1000));

    assert_eq!(CHECK.load(Ordering::SeqCst), 4);

    assert_eq!(
        ASSERT_SYN_ACK_HDR_SEQ_ACK.load(Ordering::SeqCst),
        SYN_SEQ_RANDOM_A + 1
    );
    assert_eq!(
        ASSERT_ACK_HDR_SEQ_SYN.load(Ordering::SeqCst),
        SYN_SEQ_RANDOM_A + 1
    );
    assert_eq!(
        ASSERT_ACK_HDR_SEQ_ACK.load(Ordering::SeqCst),
        SYN_ACK_SEQ_RANDOM_B + 1
    );

    std::thread::sleep(std::time::Duration::from_millis(1000));

    assert_eq!(ASSERT_PUSH_PAYLOAD.load(Ordering::SeqCst), PUSH_PAYLOAD);
}
