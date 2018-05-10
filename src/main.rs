#![feature(plugin)]
#![feature(custom_derive)]
#![plugin(rocket_codegen)]

extern crate dotenv;
extern crate rocket;
#[macro_use] extern crate rocket_contrib;
extern crate image2aa;
extern crate rusoto_core;
extern crate rusoto_credential;
extern crate rusoto_s3;
extern crate image;
extern crate time;
extern crate sha3;

use std::io;
use std::io::Read;
use std::env;
use std::sync::mpsc;
use std::sync::mpsc::SyncSender;
use std::thread;
use rocket_contrib::Json;
use rocket::response::NamedFile;
use rocket::State;
use rusoto_s3::S3;
use sha3::{Digest, Sha3_256};
use image2aa::{filter, utils};

#[derive(FromForm)]
struct Options {
    blocksize: Option<usize>,
    char_detect_thresh: Option<u32>,
    line_detect_thresh: Option<u32>
}

struct S3Uploader {
    uploaded_image_hashes: Vec<String>
}

impl S3Uploader {
    pub fn new() -> Self {
        Self { uploaded_image_hashes: vec![] }
    }

    pub fn upload_s3(&mut self, image_buf: Vec<u8>) {
        let mut hasher = Sha3_256::default();
        hasher.input(&image_buf);
        let image_hash = format!("{:x}", hasher.result());

        if !self.uploaded_image_hashes.contains(&image_hash) {
            self.uploaded_image_hashes.push(image_hash.clone());

            let file_extension = match image::guess_format(&image_buf) {
                Ok(image::ImageFormat::PNG) => "png",
                Ok(image::ImageFormat::JPEG) => "jpg",
                _ => "unknown"
            };

            let s3 = rusoto_s3::S3Client::new(
                rusoto_core::reactor::RequestDispatcher::default(),
                rusoto_credential::EnvironmentProvider,
                rusoto_core::Region::ApNortheast1
            );
            let bucket_name = env::var("AWS_S3_BUCKET_NAME").unwrap();
            let mut put_request = rusoto_s3::PutObjectRequest::default();
            put_request.bucket = bucket_name;
            put_request.key = format!("{}_{}.{}", time::now().to_timespec().sec, image_hash, file_extension);
            put_request.body = Some(image_buf);
            s3.put_object(&put_request).sync().unwrap();
        }
    }
}

#[get("/")]
fn index() -> io::Result<NamedFile> {
    NamedFile::open("static/index.html")
}

#[post("/image", data = "<image_binary>")]
fn image_without_options(image_binary: rocket::Data, tx: State<SyncSender<Vec<u8>>>) -> Json {
    let options = Options { blocksize: None, char_detect_thresh: None, line_detect_thresh: None };
    image(options, image_binary, tx)
}

#[post("/image?<options>", data = "<image_binary>")]
fn image(options: Options, image_binary: rocket::Data, tx: State<SyncSender<Vec<u8>>>) -> Json {
    let mut image_buf = vec![];
    image_binary.open().read_to_end(&mut image_buf).unwrap();

    tx.send(image_buf.clone()).unwrap();

    let mut hough_filter = filter::hough::default();
    if let Some(block_size) = options.blocksize { hough_filter.block_size = block_size; }
    if let Some(slope_count_thresh) = options.char_detect_thresh { hough_filter.slope_count_thresh = slope_count_thresh; }

    let mut binary_filter = filter::binary::default();
    if let Some(thresh) = options.line_detect_thresh { binary_filter.thresh = thresh; }

    let image_array = utils::read_image(image_buf.as_slice()).map_err(|e| println!("{}", e)).unwrap();

    let grayscale_array = filter::grayscale::default().run(image_array);
    let gradient_array = filter::line::default().run(grayscale_array.clone());
    let line_array = binary_filter.run(gradient_array).mapv(|e| e as f32) * 250.;
    let hough_array = hough_filter.run(line_array);
    let aa = filter::ascii_art::default().run(hough_array);
    Json(json!({ "aa": aa }))
}

fn main() {
    dotenv::dotenv().ok();

    let (tx, rx) = mpsc::sync_channel(10);

    thread::spawn(move || {
        let mut uploader = S3Uploader::new();
        loop {
            let image_buf = rx.recv().unwrap();
            uploader.upload_s3(image_buf);
        }
    });

    rocket::ignite()
        .manage(tx)
        .mount("/", routes![index, image, image_without_options])
        .launch();
}
