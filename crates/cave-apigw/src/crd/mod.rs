// SPDX-License-Identifier: AGPL-3.0-or-later
//! Kubernetes Gateway API CRDs — Gateway, HTTPRoute, GRPCRoute, TLSRoute,
//! TCPRoute, UDPRoute, GatewayClass. Reference: gateway-api v1.3.0.

pub mod gateway;
pub mod httproute;
pub mod grpcroute;
pub mod tlsroute;
pub mod tcproute;
pub mod udproute;

pub use gateway::{Gateway, GatewayClass, GatewayListener};
pub use httproute::{HttpRoute, HttpRouteRule, HttpBackend};
pub use grpcroute::{GrpcRoute, GrpcRouteRule, GrpcMethodMatch};
pub use tlsroute::TlsRoute;
pub use tcproute::TcpRoute;
pub use udproute::UdpRoute;
