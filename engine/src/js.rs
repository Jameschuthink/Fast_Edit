//! One QuickJS runtime per render worker. Loads the prelude, the engine libraries the job asks for, the job data
//! (as `JOB`) and the composition, then turns `draw(g, t)` into an op record on demand.

use crate::assets::Assets;
use crate::canvas::StrTable;
use crate::text::{register_font, Shaper};
use anyhow::{anyhow, Result};
use rquickjs::{
    context::EvalOptions, function::Opt, Context, Ctx, Function, Object, Runtime, TypedArray, Value,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// Everything a worker needs to build the same composition.
#[derive(Clone)]
pub struct CompSrc {
    pub engine_js: PathBuf,
    pub libs: Vec<String>,
    pub job_json: String,
    pub files: Vec<PathBuf>,
    pub world: (u32, u32),
    pub fps: f64,
}

pub struct Js {
    _rt: Runtime,
    ctx: Context,
    pub strs: StrTable,
}

fn js_err(ctx: &Ctx, e: rquickjs::Error) -> anyhow::Error {
    if let rquickjs::Error::Exception = e {
        let v = ctx.catch();
        if let Some(ex) = v.as_exception() {
            let msg = ex.message().unwrap_or_default();
            let stack = ex.stack().unwrap_or_default();
            return anyhow!("{msg}\n{}", stack.trim_end());
        }
        if let Ok(s) = v.get::<rquickjs::String>() {
            return anyhow!("thrown: {}", s.to_string().unwrap_or_default());
        }
        return anyhow!("JS exception: {:?}", v);
    }
    anyhow!("{e}")
}

fn eval_file(ctx: &Ctx, path: &Path) -> Result<()> {
    let src = std::fs::read_to_string(path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    let mut o = EvalOptions::default();
    o.global = true;
    o.strict = false;
    o.filename = Some(path.display().to_string());
    ctx.eval_with_options::<(), _>(src, o).map_err(|e| js_err(ctx, e))
}

impl Js {
    pub fn new(src: &CompSrc, assets: Arc<Assets>, shaper: Rc<RefCell<Shaper>>) -> Result<Js> {
        let rt = Runtime::new()?;
        rt.set_memory_limit(1 << 30);
        rt.set_max_stack_size(4 << 20);
        let ctx = Context::full(&rt)?;
        ctx.with(|ctx| -> Result<()> {
            let g = ctx.globals();
            register_natives(&ctx, &g, assets.clone(), shaper.clone())?;
            eval_file(&ctx, &src.engine_js.join("prelude.js"))?;
            // JOB: the job's data, parsed by JS itself; libraries may read it
            let job: Value = ctx.json_parse(src.job_json.clone()).map_err(|e| js_err(&ctx, e))?;
            g.set("JOB", job)?;
            let begin: Function = g.get("__begin")?;
            begin.call::<_, ()>((src.world.0, src.world.1, src.fps)).map_err(|e| js_err(&ctx, e))?;
            for lib in &src.libs {
                let p = if lib.ends_with(".js") { PathBuf::from(lib) } else { src.engine_js.join("lib").join(format!("{lib}.js")) };
                eval_file(&ctx, &p)?;
            }
            for f in &src.files {
                eval_file(&ctx, f)?;
            }
            if let Ok(setup) = g.get::<_, Function>("setup") {
                setup.call::<_, Value>(()).map_err(|e| js_err(&ctx, e))?;
                while ctx.execute_pending_job() {}
            }
            Ok(())
        })?;
        Ok(Js { _rt: rt, ctx, strs: StrTable::default() })
    }

    /// Runs draw(g, t) and returns the op record. New strings go into self.strs.
    pub fn record(&mut self, t: f64) -> Result<Vec<f64>> {
        let strs = &mut self.strs;
        self.ctx.with(|ctx| -> Result<Vec<f64>> {
            let g = ctx.globals();
            let begin: Function = g.get("__frame_begin")?;
            begin.call::<_, ()>((t,)).map_err(|e| js_err(&ctx, e))?;
            while ctx.execute_pending_job() {}
            let end: Function = g.get("__frame_end")?;
            let r: rquickjs::Array = end.call(()).map_err(|e| js_err(&ctx, e))?;
            let n: usize = r.get(0)?;
            let news: rquickjs::Array = r.get(1)?;
            for i in 0..news.len() {
                let s: String = news.get(i)?;
                strs.push(s);
            }
            let ops_fn: Function = g.get("__ops")?;
            let arr: TypedArray<f64> = ops_fn.call(())?;
            // SAFETY: no JS runs while the slice is alive; it is copied out at once
            let slice: &[f64] = unsafe { arr.as_slice() };
            Ok(slice[..n.min(slice.len())].to_vec())
        })
        .map_err(|e| anyhow!("t={t:.3}: {e}"))
    }

    /// Evaluates an expression and returns it as JSON (e.g. "typeof EVENTS !== 'undefined' ? EVENTS : []").
    pub fn eval_json(&self, expr: &str) -> Result<serde_json::Value> {
        self.ctx.with(|ctx| -> Result<serde_json::Value> {
            let v: Value = ctx.eval(format!("JSON.stringify(({expr}))")).map_err(|e| js_err(&ctx, e))?;
            if v.is_undefined() || v.is_null() {
                return Ok(serde_json::Value::Null);
            }
            let s: String = v.get().map_err(|e| js_err(&ctx, e))?;
            Ok(serde_json::from_str(&s).unwrap_or(serde_json::Value::Null))
        })
    }
}

fn register_natives<'js>(ctx: &Ctx<'js>, g: &Object<'js>, assets: Arc<Assets>, shaper: Rc<RefCell<Shaper>>) -> Result<()> {
    g.set("__log", Function::new(ctx.clone(), |s: String| {
        eprintln!("[js] {s}");
    })?)?;
    let a = assets.clone();
    g.set("__img", Function::new(ctx.clone(), move |ctx: Ctx<'js>, p: String| -> rquickjs::Result<Vec<f64>> {
        match a.image(&p) {
            Ok((id, w, h)) => Ok(vec![id as f64, w as f64, h as f64]),
            Err(e) => Err(rquickjs::Exception::throw_message(&ctx, &format!("img('{p}'): {e}"))),
        }
    })?)?;
    let a = assets.clone();
    g.set("__video", Function::new(ctx.clone(), move |ctx: Ctx<'js>, p: String, o: Opt<String>| -> rquickjs::Result<Vec<f64>> {
        match a.video(&p, o.0.as_deref().unwrap_or("{}")) {
            Ok((id, w, h, fps, n)) => Ok(vec![id as f64, w as f64, h as f64, fps, n as f64]),
            Err(e) => Err(rquickjs::Exception::throw_message(&ctx, &format!("video('{p}'): {e}"))),
        }
    })?)?;
    let a = assets.clone();
    g.set("__seq", Function::new(ctx.clone(), move |ctx: Ctx<'js>, p: String, n: usize| -> rquickjs::Result<Vec<f64>> {
        match a.seq(&p, n) {
            Ok((id, w, h, first, n)) => Ok(vec![id as f64, w as f64, h as f64, first as f64, n as f64]),
            Err(e) => Err(rquickjs::Exception::throw_message(&ctx, &format!("seq('{p}'): {e}"))),
        }
    })?)?;
    let a = assets.clone();
    g.set("__font", Function::new(ctx.clone(), move |ctx: Ctx<'js>, fam: String, file: String, o: Opt<String>| -> rquickjs::Result<()> {
        let path = a.resolve(&file);
        let opts: serde_json::Value = o.0.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
        let weight = opts["weight"].as_i64().map(|w| w as i32);
        let italic = opts["style"].as_str().map(|s| s == "italic");
        let mut pins = Vec::new();
        if let Some(m) = opts["pins"].as_object() {
            for (k, v) in m {
                if let Some(f) = v.as_f64() {
                    pins.push((k.clone(), f as f32));
                }
            }
        }
        register_font(&fam, &path.to_string_lossy(), weight, italic, &pins)
            .map_err(|e| rquickjs::Exception::throw_message(&ctx, &format!("font('{fam}'): {e}")))
    })?)?;
    let s = shaper.clone();
    g.set("__measure", Function::new(ctx.clone(), move |font: String, ls: f64, text: String| -> Vec<f64> {
        let sh = s.borrow_mut().shape(&font, ls as f32, 0., &text, None);
        let ink = sh.ink;
        let (l, r, asc, desc) = if ink.is_empty() { (0., 0., 0., 0.) } else { (-ink.left, ink.right, -ink.top, ink.bottom) };
        vec![sh.width as f64, l as f64, r as f64, asc as f64, desc as f64, sh.ascent as f64, sh.descent as f64]
    })?)?;
    let s = shaper.clone();
    g.set("__measure_box", Function::new(ctx.clone(), move |font: String, ls: f64, text: String, w: f64, lh: f64| -> Vec<f64> {
        let sh = s.borrow_mut().shape(&font, ls as f32, 0., &text, Some((w as f32, 0, lh as f32)));
        vec![sh.lines as f64, sh.height as f64, sh.width as f64]
    })?)?;
    Ok(())
}
