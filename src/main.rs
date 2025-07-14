use ntex::web::{self, middleware, App, HttpRequest, HttpResponse, HttpServer};
use ntex::http::body::Body; // Correctly import the Body type
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use parking_lot::RwLock;
use bytes::Bytes;

// --- Application State: File Cache ---
#[derive(Clone)]
struct FileCache {
    cache: Arc<RwLock<HashMap<PathBuf, Arc<Bytes>>>>,
    static_root: PathBuf,
}

impl FileCache {
    pub fn new(static_root: &str) -> Self {
        FileCache {
            cache: Arc::new(RwLock::new(HashMap::new())),
            static_root: PathBuf::from(static_root),
        }
    }

    pub async fn get_or_load(&self, path: &Path) -> Result<Arc<Bytes>, std::io::Error> {
        if let Some(file_bytes) = self.cache.read().get(path) {
            return Ok(file_bytes.clone());
        }

        let mut cache_writer = self.cache.write();
        if let Some(file_bytes) = cache_writer.get(path) {
            return Ok(file_bytes.clone());
        }

        let full_path = self.static_root.join(path);
        log::info!("Cache miss, loading file from disk: {:?}", full_path);

        // This now works because `glommio` is a direct dependency
        let file_contents = glommio::io::read_all(full_path).await?;
        let file_bytes = Arc::new(Bytes::from(file_contents));

        cache_writer.insert(path.to_path_buf(), file_bytes.clone());
        Ok(file_bytes)
    }
}

// --- Route Handlers ---

async fn health_handler() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/plain; charset=utf-8")
        .body("OK")
}

async fn static_file_handler(
    req: HttpRequest,
    cache: web::types::State<FileCache>,
) -> Result<HttpResponse, std::io::Error> {
    let path_str: String = req.match_info().query("filename").parse().unwrap_or_else(|_| "index.html".to_string());
    let path = if path_str.is_empty() { Path::new("index.html") } else { Path::new(&path_str) };


    match cache.get_or_load(path).await {
        Ok(file_bytes) => {
            let content_type = mime_guess::from_path(path).first_or_octet_stream();
            Ok(HttpResponse::Ok()
                .content_type(content_type.as_ref())
                // Use the imported `Body` type directly
                .body(Body::from(file_bytes.as_ref().clone())))
        }
        Err(e) => {
            log::warn!("Failed to load file '{:?}': {}", path, e);
            Ok(HttpResponse::NotFound().body("Not Found"))
        }
    }
}

// --- Main Application Entrypoint ---

// Use the correct main macro from the ntex-glommio crate
#[ntex_glommio::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    std::fs::create_dir_all("./static_root")?;
    std::fs::write(
        "./static_root/index.html",
        "<!DOCTYPE html><html><body><h1>Fixed io_uring Server with ntex!</h1></body></html>",
    )?;

    let cache = FileCache::new("./static_root");
    let num_cores = num_cpus::get();
    log::info!("Starting server with {num_cores} workers on http://127.0.0.1:8080");

    HttpServer::new(move || {
        App::new()
            .state(cache.clone())
            .wrap(middleware::Logger::default())
            .route("/health", web::get().to(health_handler))
            .route("/{filename:.*}", web::get().to(static_file_handler))
    })
    .bind(("127.0.0.1", 8080))?
    .workers(num_cores)
    .run()
    .await
}