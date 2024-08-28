use actix_cors::Cors;
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use anyhow::Result;

mod calendar;
use calendar::Calendar;
mod format;
use format::Format;

struct AppState {
    cal: Calendar,
}
impl AppState {
    pub fn new(cal: Calendar) -> Self {
        Self { cal }
    }

    pub async fn get_events(&self, format: Format) -> Result<String> {
        let now = chrono::Local::now();
        let events = self
            .cal
            .get_events(now - chrono::Duration::days(1), now, false)
            .await?;

        format
            .format(events)
            .ok_or(anyhow::anyhow!("No events found".to_string()))
    }
}

#[get("/")]
async fn index() -> impl Responder {
    HttpResponse::Ok().body("Hello world!")
}

#[get("/raw")]
async fn raw(data: web::Data<AppState>) -> actix_web::Result<String> {
    data.get_events(Format::Raw)
        .await
        .map_err(|err| actix_web::error::ErrorFailedDependency(err.to_string()))
}

#[get("/tana")]
async fn tana(data: web::Data<AppState>) -> actix_web::Result<String> {
    data.get_events(Format::Tana)
        .await
        .map_err(|err| actix_web::error::ErrorFailedDependency(err.to_string()))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv::dotenv().ok();

    let cal = Calendar::new().await.expect("Failed to create app state");
    let data = web::Data::new(AppState::new(cal));

    println!("🚀 Server started successfully");
    HttpServer::new(move || {
        App::new()
            .app_data(data.clone())
            .wrap(
                Cors::default()
                    .allowed_origin("https://app.tana.inc")
                    .allowed_methods(vec!["GET", "POST"]),
            )
            .service(index)
            .service(tana)
    })
    .bind(("127.0.0.1", 7117))?
    .run()
    .await
}
