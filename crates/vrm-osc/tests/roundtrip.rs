use vrm_osc::{
    OscArray, OscBundle, OscColor, OscMessage, OscMidiMessage, OscPacket, OscTime, OscType,
    decoder, encoder,
};

fn all_supported_args() -> Vec<OscType> {
    vec![
        OscType::Int(-12),
        OscType::Float(3.5),
        OscType::String("hello".to_owned()),
        OscType::Blob(vec![0, 1, 2, 3, 4]),
        OscType::Time(OscTime {
            seconds: 1,
            fractional: 2,
        }),
        OscType::Long(-9_000_000_000),
        OscType::Double(0.125),
        OscType::Char('界'),
        OscType::Color(OscColor {
            red: 1,
            green: 2,
            blue: 3,
            alpha: 4,
        }),
        OscType::Midi(OscMidiMessage {
            port: 5,
            status: 0x90,
            data1: 64,
            data2: 127,
        }),
        OscType::Bool(true),
        OscType::Bool(false),
        OscType::Array(OscArray {
            content: vec![
                OscType::Int(1),
                OscType::Array(OscArray {
                    content: vec![OscType::String("nested".to_owned()), OscType::Nil],
                }),
                OscType::Inf,
            ],
        }),
        OscType::Nil,
        OscType::Inf,
    ]
}

#[test]
fn roundtrips_message_with_all_supported_arg_types() {
    let packet = OscPacket::Message(OscMessage {
        addr: "/all".to_owned(),
        args: all_supported_args(),
    });

    let bytes = encoder::encode(&packet).unwrap();
    let (remainder, decoded) = decoder::decode_udp(&bytes).unwrap();

    assert!(remainder.is_empty());
    assert_eq!(decoded, packet);
}

#[test]
fn roundtrips_nested_bundles() {
    let packet = OscPacket::Bundle(OscBundle {
        timetag: OscTime::IMMEDIATE,
        content: vec![
            OscPacket::Message(OscMessage {
                addr: "/left".to_owned(),
                args: vec![1_i32.into()],
            }),
            OscPacket::Bundle(OscBundle {
                timetag: (10, 20).into(),
                content: vec![OscPacket::Message(OscMessage {
                    addr: "/right".to_owned(),
                    args: vec!["ok".into()],
                })],
            }),
        ],
    });

    let bytes = encoder::encode(&packet).unwrap();
    let (remainder, decoded) = decoder::decode_udp(&bytes).unwrap();

    assert!(remainder.is_empty());
    assert_eq!(decoded, packet);
}

#[test]
fn tcp_roundtrip_single_and_vec() {
    let first = OscPacket::Message(OscMessage {
        addr: "/one".to_owned(),
        args: vec![OscType::Float(1.0)],
    });
    let second = OscPacket::Message(OscMessage {
        addr: "/two".to_owned(),
        args: vec![OscType::Float(2.0)],
    });

    let mut bytes = encoder::encode_tcp(&first).unwrap();
    bytes.extend_from_slice(&encoder::encode_tcp(&second).unwrap());

    let (remainder, packets) = decoder::decode_tcp_vec(&bytes).unwrap();
    assert!(remainder.is_empty());
    assert_eq!(packets, vec![first, second]);
}

#[test]
fn encode_string_uses_osc_padding() {
    assert_eq!(encoder::encode_string(""), vec![0, 0, 0, 0]);
    assert_eq!(encoder::encode_string("a"), vec![b'a', 0, 0, 0]);
    assert_eq!(
        encoder::encode_string("abcd"),
        vec![b'a', b'b', b'c', b'd', 0, 0, 0, 0]
    );
    assert_eq!(encoder::pad(10), 12);
}

#[test]
fn bad_type_tag_is_rejected() {
    let mut bytes = encoder::encode_string("/bad");
    bytes.extend_from_slice(&encoder::encode_string(",x"));
    let err = decoder::decode_udp(&bytes).unwrap_err();
    assert!(err.to_string().contains("not implemented"));
}
