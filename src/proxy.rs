use std::{future::Future, net::SocketAddr, pin::Pin};

use http_body_util::BodyExt;
use hyper::{body::Incoming, service::Service, Request};
use tokio::net::TcpStream;

use crate::{
    config::Config,
    request::ProxyRequest,
    response::{BoxBodyResponse, LocalResponse, ProxyResponse},
};

/// Proxy service. Handles incoming requests from clients and responses from
/// target servers.
pub(crate) struct Proxy {
    /// Reference to global config.
    config: &'static Config,
    client_addr: SocketAddr,
    server_addr: SocketAddr,
}

impl Proxy {
    /// Creates a new [`Proxy`].
    pub fn new(config: &'static Config, client_addr: SocketAddr, server_addr: SocketAddr) -> Self {
        Self {
            config,
            client_addr,
            server_addr,
        }
    }

    /// Forwards the request to the target server and returns the response sent
    /// by the target server. See [`ProxyRequest`] and [`ProxyResponse`].
    pub async fn forward(
        request: ProxyRequest<Incoming>,
        to: SocketAddr,
    ) -> Result<BoxBodyResponse, hyper::Error> {
        let stream = TcpStream::connect(to).await.unwrap();

        let (mut sender, conn) = hyper::client::conn::http1::Builder::new()
            .preserve_header_case(true)
            .title_case_headers(true)
            .handshake(stream)
            .await?;

        tokio::task::spawn(async move {
            if let Err(err) = conn.await {
                println!("Connection failed: {:?}", err);
            }
        });

        let response = sender.send_request(request.into_forwarded()).await?;

        Ok(ProxyResponse::new(response.map(|body| body.boxed())).into_forwarded())
    }
}

impl Service<Request<Incoming>> for Proxy {
    type Response = BoxBodyResponse;

    type Error = hyper::Error;

    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&mut self, request: Request<Incoming>) -> Self::Future {
        let Proxy {
            config,
            client_addr,
            server_addr,
        } = *self;

        Box::pin(async move {
            if !request.uri().to_string().starts_with(&config.prefix) {
                Ok(LocalResponse::not_found())
            } else {
                let request = ProxyRequest::new(request, client_addr, server_addr);
                Proxy::forward(request, config.target).await
            }
        })
    }
}