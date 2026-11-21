import asyncio
import json

import httpx
from websockets.asyncio.server import serve

from donatehub.collectors.signalr import RS, decode, encode, signalr_stream


def test_encode_appends_record_separator():
    assert encode({"type": 6}) == '{"type":6}\x1e'


def test_decode_multiple_frames():
    msgs = decode('{"type":6}\x1e{"type":1,"target":"X","arguments":[1]}\x1e')
    assert msgs == [{"type": 6}, {"type": 1, "target": "X", "arguments": [1]}]


async def test_stream_handshake_and_invocations():
    seen = {}

    async def hub(ws):
        seen["path"] = ws.request.path
        seen["handshake"] = decode(await ws.recv())
        await ws.send("{}" + RS)
        await ws.send(encode({"type": 6}) + encode({"type": 1, "target": "DonationCreated", "arguments": [{"id": "a"}]}))
        await ws.send(encode({"type": 7, "error": "bye"}))
        await asyncio.sleep(1)

    def negotiate(request):
        seen["negotiate"] = str(request.url)
        return httpx.Response(200, json={"connectionToken": "CT", "negotiateVersion": 1})

    async with serve(hub, "127.0.0.1", 0) as server:
        port = server.sockets[0].getsockname()[1]
        client = httpx.AsyncClient(transport=httpx.MockTransport(negotiate))
        got = []
        try:
            async def opened():
                got.append("open")

            async for target, args in signalr_stream(f"http://127.0.0.1:{port}/hub", "tok", client, on_open=opened):
                got.append((target, args))
        except ConnectionError as e:
            assert "bye" in str(e)
        else:
            raise AssertionError("expected ConnectionError on close message")

    assert got == ["open", ("DonationCreated", [{"id": "a"}])]
    assert seen["handshake"] == [{"protocol": "json", "version": 1}]
    assert "/hub/negotiate?negotiateVersion=1" in seen["negotiate"]
    assert "access_token=tok" in seen["negotiate"]
    assert seen["path"].startswith("/hub?") and "id=CT" in seen["path"] and "access_token=tok" in seen["path"]
