#![feature(plugin)]
#![feature(proc_macro_derive)]

extern crate dotenv;
#[macro_use]
extern crate rocket;
extern crate image;
extern crate image2aa;
extern crate image2aa_web;
extern crate rand;
extern crate sha3;
extern crate time;

use image2aa::{filter, utils};
use rand::Rng;
use rocket::http::Status;
use rocket::request::LenientForm;
use rocket::response::{NamedFile, Responder};
use rocket::tokio::io::AsyncReadExt;
use rocket::{Request, Response, State};
use rocket_contrib::Json;
use sha3::Sha3_256;
use std::env;
use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::sync::mpsc::SyncSender;
use std::thread;

struct ContentDisposition(NamedFile, String);

impl<'r, 'o: 'r> Responder<'r, 'o> for ContentDisposition {
    fn respond_to(self, request: &Request) -> Result<Response<'r>, Status> {
        let filename = self.1.clone();
        match self.0.respond_to(request) {
            Ok(mut response) => {
                response.adjoin_raw_header(
                    "Content-Disposition",
                    format!("attachment; filename=\"{}\"", filename),
                );
                Ok(response)
            }
            Err(status) => Err(status),
        }
    }
}

#[derive(FromForm)]
struct Options {
    blocksize: Option<usize>,
    char_detect_thresh: Option<u32>,
    line_detect_thresh: Option<u32>,
}

#[derive(FromForm)]
struct AsciiArtForm {
    text: String,
}

struct S3Uploader {
    uploaded_image_hashes: Vec<String>,
}

#[get("/")]
fn index() -> io::Result<NamedFile> {
    NamedFile::open("static/index.html")
}

#[post("/download_aa_image", data = "<ascii_art>")]
fn download_aa_image(ascii_art: LenientForm<AsciiArtForm>) -> io::Result<ContentDisposition> {
    println!("hoge");
    let filename = format!(
        "/tmp/{}_{}.png",
        time::now().to_timespec().sec,
        rand::thread_rng()
            .gen_ascii_chars()
            .take(20)
            .collect::<String>()
    );
    let path = Path::new(&filename);
    let image = image2aa_web::text2image(ascii_art.get().text.clone());
    image.save(path.clone()).unwrap();
    NamedFile::open(&filename).map(|named_file| {
        ContentDisposition(
            named_file,
            path.file_name().unwrap().to_string_lossy().to_string(),
        )
    })
}

#[post("/image", data = "<image_binary>")]
fn image_without_options(image_binary: rocket::Data) -> Json {
    let options = Options {
        blocksize: None,
        char_detect_thresh: None,
        line_detect_thresh: None,
    };
    image(options, image_binary)
}

#[post("/image?<options>", data = "<image_binary>")]
fn image(options: Options, image_binary: rocket::Data) -> Json {
    let mut image_buf = vec![];
    image_binary.open().read_to_end(&mut image_buf).await;

    let mut hough_filter = filter::hough::default();
    if let Some(block_size) = options.blocksize {
        hough_filter.block_size = block_size;
    }
    if let Some(slope_count_thresh) = options.char_detect_thresh {
        hough_filter.slope_count_thresh = slope_count_thresh;
    }

    let mut binary_filter = filter::binary::default();
    if let Some(thresh) = options.line_detect_thresh {
        binary_filter.thresh = thresh;
    }

    let image_array = utils::read_image(image_buf.as_slice())
        .map_err(|e| println!("{}", e))
        .unwrap();

    let grayscale_array = filter::grayscale::default().run(image_array);
    let gradient_array = filter::line::default().run(grayscale_array.clone());
    let line_array = binary_filter.run(gradient_array).mapv(|e| e as f32) * 250.;
    let hough_array = hough_filter.run(line_array);
    let aa = filter::ascii_art::default().run(hough_array);
    Json(json!({ "aa": aa }))
}

fn main() {
    dotenv::dotenv().ok();

    rocket::ignite()
        .mount(
            "/",
            routes![index, image, image_without_options, download_aa_image],
        )
        .launch();
}
