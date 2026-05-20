use bytes::Bytes;

pub use aether_contracts::tunnel::{
    decode_payload, encode_frame, encode_goaway, encode_ping, encode_pong, encode_stream_error,
    frame_payload_by_header, FrameHeader, RequestMeta, ResponseMeta, FLAG_END_STREAM,
    FLAG_GZIP_COMPRESSED, GOAWAY, HEADER_SIZE, HEARTBEAT_ACK, HEARTBEAT_DATA, PING, PONG,
    REQUEST_BODY, REQUEST_HEADERS, RESPONSE_BODY, RESPONSE_HEADERS, STREAM_END, STREAM_ERROR,
};

pub fn compress_payload(payload: &[u8]) -> Result<(Vec<u8>, u8), std::io::Error> {
    let (compressed, flags) =
        aether_contracts::tunnel::compress_payload(Bytes::copy_from_slice(payload));
    Ok((compressed.to_vec(), flags))
}

pub fn raw_payload(payload: &[u8]) -> (Vec<u8>, u8) {
    let (payload, flags) = aether_contracts::tunnel::raw_payload(Bytes::copy_from_slice(payload));
    (payload.to_vec(), flags)
}
