#![deny(warnings)]
extern crate hyper;
extern crate futures;
extern crate tokio_core;

use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use hyper::client::{Client, Request, DefaultConnector};
use hyper::{Method, StatusCode};

use futures::Future;
use futures::sync::oneshot;

use tokio_core::reactor::{Core, Handle};

fn client(handle: &Handle) -> Client<DefaultConnector> {
    Client::new(handle).unwrap()
}

fn s(buf: &[u8]) -> &str {
    ::std::str::from_utf8(buf).unwrap()
}

macro_rules! test {
    (
        name: $name:ident,
        server:
            expected: $server_expected:expr,
            reply: $server_reply:expr,
        client:
            request:
                method: $client_method:ident,
                url: $client_url:expr,
                headers: [ $($request_headers:expr,)* ],
                body: $request_body:expr,

            response:
                status: $client_status:ident,
                headers: [ $($response_headers:expr,)* ],
                body: $response_body:expr,
    ) => (
        #[test]
        fn $name() {
            #![allow(unused)]
            use hyper::header::*;
            let server = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = server.local_addr().unwrap();
            let mut core = Core::new().unwrap();
            let client = client(&core.handle());
            let mut req = Request::new(Method::$client_method, format!($client_url, addr=addr).parse().unwrap());
            $(
                req.headers_mut().set($request_headers);
            )*

            if let Some(body) = $request_body {
                let body: &'static str = body;
                req.set_body(body);
            }
            let res = client.request(req);

            let (tx, rx) = oneshot::channel();

            thread::spawn(move || {
                let mut inc = server.accept().unwrap().0;
                inc.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                inc.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
                let expected = format!($server_expected, addr=addr);
                let mut buf = [0; 4096];
                let mut n = 0;
                while n < buf.len() && n < expected.len() {
                    n += inc.read(&mut buf[n..]).unwrap();
                }
                assert_eq!(s(&buf[..n]), expected);

                inc.write_all($server_reply.as_ref()).unwrap();
                tx.complete(());
            });

            let rx = rx.map_err(|_| hyper::Error::Io(io::Error::new(io::ErrorKind::Other, "thread panicked")));

            let work = res.join(rx).map(|r| r.0);

            let res = core.run(work).unwrap();
            assert_eq!(res.status(), &StatusCode::$client_status);
            $(
                assert_eq!(res.headers().get(), Some(&$response_headers));
            )*
        }
    );
}

static REPLY_OK: &'static str = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";

test! {
    name: client_get,

    server:
        expected: "GET / HTTP/1.1\r\nHost: {addr}\r\n\r\n",
        reply: REPLY_OK,

    client:
        request:
            method: Get,
            url: "http://{addr}/",
            headers: [],
            body: None,
        response:
            status: Ok,
            headers: [
                ContentLength(0),
            ],
            body: None,
}

test! {
    name: client_get_query,

    server:
        expected: "GET /foo?key=val HTTP/1.1\r\nHost: {addr}\r\n\r\n",
        reply: REPLY_OK,

    client:
        request:
            method: Get,
            url: "http://{addr}/foo?key=val#dont_send_me",
            headers: [],
            body: None,
        response:
            status: Ok,
            headers: [
                ContentLength(0),
            ],
            body: None,
}

test! {
    name: client_post_sized,

    server:
        expected: "\
            POST /length HTTP/1.1\r\n\
            Host: {addr}\r\n\
            Content-Length: 7\r\n\
            \r\n\
            foo bar\
            ",
        reply: REPLY_OK,

    client:
        request:
            method: Post,
            url: "http://{addr}/length",
            headers: [
                ContentLength(7),
            ],
            body: Some("foo bar"),
        response:
            status: Ok,
            headers: [],
            body: None,
}

test! {
    name: client_post_chunked,

    server:
        expected: "\
            POST /chunks HTTP/1.1\r\n\
            Host: {addr}\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            B\r\n\
            foo bar baz\r\n\
            0\r\n\r\n\
            ",
        reply: REPLY_OK,

    client:
        request:
            method: Post,
            url: "http://{addr}/chunks",
            headers: [
                TransferEncoding::chunked(),
            ],
            body: Some("foo bar baz"),
        response:
            status: Ok,
            headers: [],
            body: None,
}

//TODO: enable once client connection pooling is working
#[ignore]
#[test]
fn client_keep_alive() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let mut core = Core::new().unwrap();
    let client = client(&core.handle());


    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").expect("write 1");

        sock.read(&mut buf).expect("read 2");
        let second_get = b"GET /b HTTP/1.1\r\n";
        assert_eq!(&buf[..second_get.len()], second_get);
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").expect("write 2");
    });

    let req = client.get(format!("http://{}/a", addr).parse().unwrap());
    core.run(req).unwrap();

    let req = client.get(format!("http://{}/b", addr).parse().unwrap());
    core.run(req).unwrap();
}