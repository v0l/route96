use std::path::PathBuf;
use std::time::Instant;

use anyhow::{bail, Result};
use candle_core::{DType, Device, Module, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::{siglip, vit};

enum Classifier {
    Vit(vit::Model),
    Siglip(siglip::VisionModel, candle_nn::Linear),
}

impl Classifier {
    fn load(kind: &str, dir: &PathBuf, device: &Device) -> Result<Self> {
        let config: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)?;
        let classes = config["id2label"].as_object().map(|m| m.len()).unwrap_or(0);
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], DType::F32, device)? };
        Ok(match kind {
            "vit" => {
                let cfg: vit::Config = serde_json::from_value(config)?;
                Classifier::Vit(vit::Model::new(&cfg, classes, vb)?)
            }
            "siglip" => {
                let cfg: siglip::VisionConfig = serde_json::from_value(config)?;
                let backbone = siglip::VisionModel::new(&cfg, true, vb.pp("vision_model"))?;
                let head = candle_nn::linear(cfg.hidden_size, classes, vb.pp("classifier"))?;
                Classifier::Siglip(backbone, head)
            }
            _ => bail!("kind must be vit or siglip"),
        })
    }

    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        Ok(match self {
            Classifier::Vit(m) => m.forward(xs)?,
            Classifier::Siglip(b, h) => b.forward(xs)?.apply(h)?,
        })
    }
}

fn text_bench(dir: &PathBuf, fixture: &str, device: &Device) -> Result<()> {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)?;
    let cfg: siglip::TextConfig = serde_json::from_value(config["text_config"].clone())?;
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], DType::F32, device)? };
    let load_started = Instant::now();
    let model = siglip::TextModel::new(&cfg, vb.pp("text_model"))?;
    let load_seconds = load_started.elapsed().as_secs_f64();
    let tensors = candle_core::safetensors::load(fixture, device)?;
    let ids = tensors["input_ids"].to_dtype(DType::U32)?;
    model.forward(&ids.narrow(0, 0, 2)?)?;
    let started = Instant::now();
    let got = model.forward(&ids)?;
    let text_seconds = started.elapsed().as_secs_f64();
    let count = ids.dim(0)? as f64;
    let diff = (&got - &tensors["features"])?.abs()?.flatten_all()?.max(0)?.to_scalar::<f32>()?;
    let labels = &tensors["labels"];
    let scale = tensors["scale"].to_vec1::<f32>()?[0] as f64;
    let bias = tensors["bias"].to_vec1::<f32>()?[0] as f64;
    let image = Tensor::randn(0f32, 1f32, (1, labels.dim(1)?), device)?;
    let score = |x: &Tensor| -> Result<Tensor> {
        let norm = x.sqr()?.sum_keepdim(1)?.sqrt()?;
        let x = x.broadcast_div(&norm)?;
        Ok(candle_nn::ops::sigmoid(&((x.matmul(&labels.t()?)? * scale)? + bias)?)?)
    };
    score(&image)?;
    let rounds = 200;
    let started = Instant::now();
    for _ in 0..rounds {
        score(&image)?.to_vec2::<f32>()?;
    }
    let score_ms = 1000.0 * started.elapsed().as_secs_f64() / rounds as f64;
    let threads = candle_core::utils::get_num_threads();
    println!(
        "{{\"kind\": \"text\", \"threads\": {threads}, \"load_seconds\": {load_seconds:.2}, \"labels_embedded\": {count}, \"labels_per_second\": {:.1}, \"check_max_abs_diff\": {diff}, \"vocabulary\": {}, \"score_ms_per_image\": {score_ms:.3}}}",
        count / text_seconds,
        labels.dim(0)?
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        bail!("usage: taxonomy-candle-bench <vit|siglip|text> <checkpoint dir> [check.safetensors [save.safetensors]]");
    }
    let device = Device::Cpu;
    if args[1] == "text" {
        return text_bench(&PathBuf::from(&args[2]), &args[3], &device);
    }
    let model = Classifier::load(&args[1], &PathBuf::from(&args[2]), &device)?;
    if let Some(check) = args.get(3) {
        let tensors = candle_core::safetensors::load(check, &device)?;
        let xs = &tensors["x"];
        model.forward(&xs.narrow(0, 0, 2)?)?;
        let started = Instant::now();
        let got = model.forward(xs)?;
        let seconds = started.elapsed().as_secs_f64();
        if let Some(save) = args.get(4) {
            candle_core::safetensors::save(&[("logits".to_string(), got.clone())].into_iter().collect(), save)?;
        }
        let images = xs.dim(0)? as f64;
        let diff = (got - &tensors["logits"])?.abs()?.flatten_all()?.max(0)?.to_scalar::<f32>()?;
        let threads = candle_core::utils::get_num_threads();
        println!(
            "{{\"kind\": \"{}\", \"device\": \"candle cpu\", \"threads\": {threads}, \"batch\": {images}, \"check_max_abs_diff\": {diff}, \"images_per_second\": {:.2}, \"images_per_second_per_core\": {:.2}}}",
            args[1],
            images / seconds,
            images / seconds / threads as f64
        );
        return Ok(());
    }
    let threads = candle_core::utils::get_num_threads();
    for batch in [1usize, 8] {
        let xs = Tensor::randn(0f32, 1f32, (batch, 3, 224, 224), &device)?;
        model.forward(&xs)?;
        let rounds = if batch == 1 { 10 } else { 3 };
        let started = Instant::now();
        for _ in 0..rounds {
            model.forward(&xs)?;
        }
        let seconds = started.elapsed().as_secs_f64();
        let images = (batch * rounds) as f64;
        println!(
            "{{\"kind\": \"{}\", \"device\": \"candle cpu\", \"threads\": {threads}, \"batch\": {batch}, \"images_per_second\": {:.2}, \"ms_per_image\": {:.1}}}",
            args[1],
            images / seconds,
            1000.0 * seconds / images
        );
    }
    Ok(())
}
