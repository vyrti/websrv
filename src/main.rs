use ntex::web::{self, App, HttpRequest, HttpResponse, HttpServer};
use ntex::http::header;
use ntex::util::Bytes;
use std::sync::Arc;
use std::collections::HashMap;

// Pre-allocated static responses for maximum performance
static OK_RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 13\r\nConnection: keep-alive\r\n\r\nHello, World!";
static JSON_RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 27\r\nConnection: keep-alive\r\n\r\n{\"message\":\"Hello, World!\"}";

// Static response cache for common paths
struct ResponseCache {
    cache: HashMap<&'static str, Bytes>,
}

impl ResponseCache {
    fn new() -> Self {
        let mut cache = HashMap::new();
        cache.insert("/", Bytes::from_static(OK_RESPONSE));
        cache.insert("/json", Bytes::from_static(JSON_RESPONSE));
        cache.insert("/ping", Bytes::from_static(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: keep-alive\r\n\r\npong"));
        
        Self { cache }
    }
    
    fn get(&self, path: &str) -> Option<&Bytes> {
        self.cache.get(path)
    }
}

// Ultra-fast handlers with minimal allocations
async fn hello(_: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .set_header(header::CONTENT_TYPE, "text/plain")
        .set_header(header::CONNECTION, "keep-alive")
        .body("Hello, World!")
}

async fn json(_: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .set_header(header::CONTENT_TYPE, "application/json")
        .set_header(header::CONNECTION, "keep-alive")
        .body(r#"{"message":"Hello, World!"}"#)
}

async fn ping(_: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .set_header(header::CONTENT_TYPE, "text/plain")
        .set_header(header::CONNECTION, "keep-alive")
        .body("pong")
}

// Health check endpoint
async fn health(_: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .set_header(header::CONTENT_TYPE, "application/json")
        .set_header(header::CONNECTION, "keep-alive")
        .body(r#"{"status":"ok"}"#)
}

// Echo endpoint for testing
async fn echo(req: HttpRequest) -> HttpResponse {
    let body = format!("Echo: {}", req.uri());
    HttpResponse::Ok()
        .set_header(header::CONTENT_TYPE, "text/plain")
        .set_header(header::CONNECTION, "keep-alive")
        .body(body)
}

#[ntex::main]
async fn main() -> std::io::Result<()> {
    // Initialize logging (optional, remove for max performance)
    env_logger::init();
    
    // Pre-allocate response cache
    let _cache = Arc::new(ResponseCache::new());
    
    println!("Starting high-performance ntex server with io_uring...");
    println!("Server will be available at http://0.0.0.0:8080");
    println!("Endpoints:");
    println!("  GET /        - Hello World");
    println!("  GET /json    - JSON response");
    println!("  GET /ping    - Ping/pong");
    println!("  GET /health  - Health check");
    println!("  GET /echo    - Echo request");
    
    HttpServer::new(|| {
        App::new()
            // Remove default logger middleware for maximum performance
            // .wrap(middleware::Logger::default())
            
            // Add compression middleware (optional, may reduce throughput)
            // .wrap(middleware::Compress::default())
            
            // Define routes with minimal overhead
            .route("/", web::get().to(hello))
            .route("/json", web::get().to(json))
            .route("/ping", web::get().to(ping))
            .route("/health", web::get().to(health))
            .route("/echo", web::get().to(echo))
            
            // Catch-all route for 404s
            .default_service(web::route().to(|| async {
                HttpResponse::NotFound()
                    .set_header(header::CONTENT_TYPE, "text/plain")
                    .body("Not Found")
            }))
    })
    .bind("0.0.0.0:8080")?
    .workers(num_cpus::get()) // Use all available CPU cores
    .run()
    .await
}