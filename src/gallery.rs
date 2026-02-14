use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

use crate::metadata::Metadata;
use crate::sort::is_photo;

/// Collect all photo relative paths from the output directory, grouped by year.
pub fn collect_photos(dir: &Path) -> HashMap<String, Vec<String>> {
    let mut by_year: HashMap<String, Vec<String>> = HashMap::new();

    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && is_photo(e.path()))
    {
        let rel = entry
            .path()
            .strip_prefix(dir)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .to_string();
        // Year is the first path component
        if let Some(year) = rel.split('/').next() {
            if year.len() == 4 && year.chars().all(|c| c.is_ascii_digit()) {
                by_year.entry(year.to_string()).or_default().push(rel);
            }
        }
    }

    // Sort files within each year
    for files in by_year.values_mut() {
        files.sort();
    }

    by_year
}

/// Build the full HTML gallery string.
pub fn generate_html(photos_by_year: &HashMap<String, Vec<String>>, metadata: &Metadata) -> String {
    let mut years: Vec<&String> = photos_by_year.keys().collect();
    years.sort();

    // Collect all tags for the filter sidebar
    let mut all_tags: Vec<String> = metadata
        .files
        .values()
        .flat_map(|info| info.tags.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    all_tags.sort();

    // Build photo entries as JSON for the JS
    let mut photo_entries = Vec::new();
    for year in &years {
        if let Some(files) = photos_by_year.get(*year) {
            for file in files {
                let info = metadata.files.get(file).cloned().unwrap_or_default();
                let tags_json: Vec<String> = info.tags.iter().map(|t| format!("\"{}\"", escape_js(t))).collect();
                let rating = info.rating.unwrap_or(0);
                photo_entries.push(format!(
                    "{{\"src\":\"{}\",\"year\":\"{}\",\"name\":\"{}\",\"tags\":[{}],\"rating\":{}}}",
                    escape_js(file),
                    escape_js(year),
                    escape_js(file.rsplit('/').next().unwrap_or(file)),
                    tags_json.join(","),
                    rating
                ));
            }
        }
    }

    let photos_json = format!("[{}]", photo_entries.join(","));

    let total_count: usize = photos_by_year.values().map(|v| v.len()).sum();

    // Build HTML grid sections
    let mut grid_html = String::new();
    for year in &years {
        if let Some(files) = photos_by_year.get(*year) {
            grid_html.push_str(&format!(
                "<h2 class=\"year-header\" data-year=\"{year}\">{year} <span class=\"count\">{}</span></h2>\n<div class=\"grid\" data-year=\"{year}\">\n",
                files.len()
            ));
            for (i, file) in files.iter().enumerate() {
                let info = metadata.files.get(file).cloned().unwrap_or_default();
                let tags_attr: String = info.tags.join(",");
                let rating = info.rating.unwrap_or(0);
                let name = file.rsplit('/').next().unwrap_or(file);
                grid_html.push_str(&format!(
                    "  <div class=\"thumb\" data-idx=\"{}\" data-tags=\"{}\" data-rating=\"{}\">\
                    <img src=\"{}\" alt=\"{}\" loading=\"lazy\"><div class=\"info\">{}</div></div>\n",
                    photo_entries.len() - total_count + i, // Not needed, we use global index from JS
                    escape_html(&tags_attr),
                    rating,
                    escape_html(file),
                    escape_html(name),
                    escape_html(name)
                ));
            }
            grid_html.push_str("</div>\n");
        }
    }

    // Tags filter HTML
    let mut tags_filter_html = String::new();
    if !all_tags.is_empty() {
        tags_filter_html.push_str("<div class=\"filter-group\"><span class=\"filter-label\">Tags</span><div class=\"filter-tags\">");
        tags_filter_html.push_str("<button class=\"tag-btn active\" data-tag=\"\">Tous</button>");
        for tag in &all_tags {
            tags_filter_html.push_str(&format!(
                "<button class=\"tag-btn\" data-tag=\"{}\">{}</button>",
                escape_html(tag),
                escape_html(tag)
            ));
        }
        tags_filter_html.push_str("</div></div>");
    }

    format!(
        r##"<!DOCTYPE html>
<html lang="fr">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>photo-sort gallery</title>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{background:#0a0a0a;color:#e0e0e0;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;overflow-x:hidden}}
header{{position:sticky;top:0;z-index:100;background:rgba(10,10,10,.95);backdrop-filter:blur(12px);padding:1rem 2rem;display:flex;justify-content:space-between;align-items:center;border-bottom:1px solid #222}}
header h1{{font-size:1.2rem;font-weight:600;color:#4fc3f7}}
.controls{{display:flex;gap:.5rem;align-items:center}}
.controls button{{background:#1a1a1a;color:#ccc;border:1px solid #333;padding:.4rem .8rem;border-radius:6px;cursor:pointer;font-size:.85rem;transition:all .2s}}
.controls button:hover,.controls button.active{{background:#4fc3f7;color:#000;border-color:#4fc3f7}}
.filter-bar{{padding:.8rem 2rem;background:#111;border-bottom:1px solid #1a1a1a;display:flex;gap:1.5rem;align-items:center;flex-wrap:wrap}}
.filter-group{{display:flex;gap:.5rem;align-items:center}}
.filter-label{{font-size:.75rem;text-transform:uppercase;color:#666;letter-spacing:.05em}}
.filter-tags{{display:flex;gap:.3rem;flex-wrap:wrap}}
.tag-btn{{background:#1a1a1a;color:#aaa;border:1px solid #2a2a2a;padding:.25rem .6rem;border-radius:12px;cursor:pointer;font-size:.8rem;transition:all .2s}}
.tag-btn:hover,.tag-btn.active{{background:#4fc3f7;color:#000;border-color:#4fc3f7}}
.rating-filter{{display:flex;gap:.2rem;align-items:center}}
.rating-filter button{{background:none;border:none;font-size:1.2rem;cursor:pointer;color:#444;transition:color .2s}}
.rating-filter button.active,.rating-filter button:hover{{color:#ffd700}}
main{{padding:1rem 2rem 4rem}}
.year-header{{margin:2rem 0 1rem;font-size:1.5rem;font-weight:300;color:#4fc3f7}}
.year-header .count{{font-size:.9rem;color:#555}}
.grid{{display:grid;grid-template-columns:repeat(auto-fill,minmax(200px,1fr));gap:6px}}
.thumb{{position:relative;aspect-ratio:1;overflow:hidden;border-radius:4px;cursor:pointer;transition:transform .2s}}
.thumb:hover{{transform:scale(1.03);z-index:1}}
.thumb img{{width:100%;height:100%;object-fit:cover}}
.thumb .info{{position:absolute;bottom:0;left:0;right:0;padding:.3rem .5rem;background:linear-gradient(transparent,rgba(0,0,0,.8));font-size:.7rem;color:#ccc;opacity:0;transition:opacity .2s}}
.thumb:hover .info{{opacity:1}}
.thumb.hidden{{display:none}}

/* Lightbox */
.lightbox{{display:none;position:fixed;inset:0;z-index:1000;background:rgba(0,0,0,.97);flex-direction:column;align-items:center;justify-content:center}}
.lightbox.open{{display:flex}}
.lightbox img{{max-width:95vw;max-height:85vh;object-fit:contain;border-radius:4px;user-select:none}}
.lb-close{{position:absolute;top:1rem;right:1.5rem;font-size:2rem;color:#888;cursor:pointer;z-index:1002;transition:color .2s}}
.lb-close:hover{{color:#fff}}
.lb-nav{{position:absolute;top:50%;transform:translateY(-50%);font-size:3rem;color:#555;cursor:pointer;user-select:none;padding:1rem;transition:color .2s}}
.lb-nav:hover{{color:#fff}}
.lb-prev{{left:1rem}}
.lb-next{{right:1rem}}
.lb-info{{margin-top:1rem;text-align:center;color:#999;font-size:.9rem}}
.lb-info .lb-name{{color:#e0e0e0;font-weight:500}}
.lb-info .lb-tags{{margin-top:.3rem}}
.lb-info .lb-tags span{{background:#1a1a1a;padding:.15rem .5rem;border-radius:10px;font-size:.75rem;margin:0 .2rem;color:#4fc3f7}}
.lb-info .lb-rating{{color:#ffd700;margin-top:.3rem;font-size:1.1rem}}
.lb-slideshow-bar{{position:absolute;bottom:0;left:0;height:3px;background:#4fc3f7;transition:width linear}}

/* Slideshow controls */
.slideshow-controls{{position:absolute;bottom:1.5rem;display:flex;gap:.5rem;z-index:1002}}
.slideshow-controls button{{background:rgba(255,255,255,.1);color:#ccc;border:1px solid #444;padding:.4rem .8rem;border-radius:6px;cursor:pointer;font-size:.85rem;transition:all .2s}}
.slideshow-controls button:hover,.slideshow-controls button.active{{background:#4fc3f7;color:#000;border-color:#4fc3f7}}

@media(max-width:600px){{
  .grid{{grid-template-columns:repeat(auto-fill,minmax(120px,1fr));gap:3px}}
  header{{padding:.8rem 1rem}}
  main{{padding:.5rem 1rem}}
}}
</style>
</head>
<body>
<header>
  <h1>photo-sort gallery</h1>
  <div class="controls">
    <button id="btn-slideshow" title="Diaporama">Diaporama</button>
    <button id="btn-random" title="Aléatoire">Aléatoire</button>
  </div>
</header>
<div class="filter-bar">
  {tags_filter}
  <div class="filter-group">
    <span class="filter-label">Note min</span>
    <div class="rating-filter" id="rating-filter">
      <button data-rating="0" class="active">&#x2715;</button>
      <button data-rating="1">&#9733;</button>
      <button data-rating="2">&#9733;</button>
      <button data-rating="3">&#9733;</button>
      <button data-rating="4">&#9733;</button>
      <button data-rating="5">&#9733;</button>
    </div>
  </div>
</div>
<main>
{grid}
</main>

<div class="lightbox" id="lightbox">
  <span class="lb-close" id="lb-close">&times;</span>
  <span class="lb-nav lb-prev" id="lb-prev">&#8249;</span>
  <span class="lb-nav lb-next" id="lb-next">&#8250;</span>
  <img id="lb-img" src="" alt="">
  <div class="lb-info">
    <div class="lb-name" id="lb-name"></div>
    <div class="lb-rating" id="lb-rating"></div>
    <div class="lb-tags" id="lb-tags"></div>
  </div>
  <div class="lb-slideshow-bar" id="lb-bar" style="width:0%"></div>
  <div class="slideshow-controls">
    <button id="ss-playpause">Pause</button>
    <button id="ss-random-toggle">Aléatoire</button>
    <button id="ss-speed-down">-</button>
    <span id="ss-speed" style="color:#ccc;font-size:.85rem">5s</span>
    <button id="ss-speed-up">+</button>
  </div>
</div>

<script>
const ALL_PHOTOS={photos_json};
let filtered=ALL_PHOTOS.slice();
let currentIdx=0;
let slideshowInterval=null;
let slideshowDelay=5000;
let slideshowRandom=false;
let activeTag="";
let minRating=0;

function applyFilters(){{
  filtered=ALL_PHOTOS.filter(p=>{{
    if(activeTag&&!p.tags.includes(activeTag))return false;
    if(minRating>0&&p.rating<minRating)return false;
    return true;
  }});
  // Update grid visibility
  document.querySelectorAll('.thumb').forEach(el=>{{
    const src=el.querySelector('img').getAttribute('src');
    const match=filtered.some(p=>p.src===src);
    el.classList.toggle('hidden',!match);
  }});
  // Update year headers
  document.querySelectorAll('.year-header').forEach(h=>{{
    const year=h.dataset.year;
    const count=filtered.filter(p=>p.year===year).length;
    h.querySelector('.count').textContent=count;
    h.style.display=count?'':'none';
    const grid=document.querySelector(`.grid[data-year="${{year}}"]`);
    if(grid)grid.style.display=count?'':'none';
  }});
}}

// Tag filter
document.querySelectorAll('.tag-btn').forEach(btn=>{{
  btn.addEventListener('click',()=>{{
    document.querySelectorAll('.tag-btn').forEach(b=>b.classList.remove('active'));
    btn.classList.add('active');
    activeTag=btn.dataset.tag;
    applyFilters();
  }});
}});

// Rating filter
document.querySelectorAll('#rating-filter button').forEach(btn=>{{
  btn.addEventListener('click',()=>{{
    document.querySelectorAll('#rating-filter button').forEach(b=>b.classList.remove('active'));
    btn.classList.add('active');
    minRating=parseInt(btn.dataset.rating);
    applyFilters();
  }});
}});

// Lightbox
const lb=document.getElementById('lightbox');
const lbImg=document.getElementById('lb-img');
const lbName=document.getElementById('lb-name');
const lbRating=document.getElementById('lb-rating');
const lbTags=document.getElementById('lb-tags');
const lbBar=document.getElementById('lb-bar');

function showPhoto(idx){{
  if(filtered.length===0)return;
  currentIdx=((idx%filtered.length)+filtered.length)%filtered.length;
  const p=filtered[currentIdx];
  lbImg.src=p.src;
  lbName.textContent=p.name+' ('+p.year+')';
  lbRating.textContent=p.rating?'★'.repeat(p.rating)+'☆'.repeat(5-p.rating):'';
  lbTags.innerHTML=p.tags.map(t=>'<span>'+t+'</span>').join('');
}}

function openLightbox(idx){{
  showPhoto(idx);
  lb.classList.add('open');
  document.body.style.overflow='hidden';
}}

function closeLightbox(){{
  lb.classList.remove('open');
  document.body.style.overflow='';
  stopSlideshow();
}}

document.getElementById('lb-close').addEventListener('click',closeLightbox);
document.getElementById('lb-prev').addEventListener('click',()=>{{showPhoto(currentIdx-1);resetSlideshowTimer();}});
document.getElementById('lb-next').addEventListener('click',()=>{{showPhoto(currentIdx+1);resetSlideshowTimer();}});

// Click on thumbnail
document.querySelectorAll('.thumb').forEach(el=>{{
  el.addEventListener('click',()=>{{
    const src=el.querySelector('img').getAttribute('src');
    const idx=filtered.findIndex(p=>p.src===src);
    if(idx>=0)openLightbox(idx);
  }});
}});

// Keyboard
document.addEventListener('keydown',e=>{{
  if(!lb.classList.contains('open'))return;
  if(e.key==='Escape')closeLightbox();
  if(e.key==='ArrowLeft'){{showPhoto(currentIdx-1);resetSlideshowTimer();}}
  if(e.key==='ArrowRight'){{showPhoto(currentIdx+1);resetSlideshowTimer();}}
}});

// Slideshow
function startSlideshow(random){{
  slideshowRandom=random;
  if(filtered.length===0)return;
  if(!lb.classList.contains('open'))openLightbox(random?Math.floor(Math.random()*filtered.length):0);
  document.getElementById('ss-random-toggle').classList.toggle('active',slideshowRandom);
  document.getElementById('ss-playpause').textContent='Pause';
  document.querySelector('.slideshow-controls').style.display='flex';
  runSlideshowTick();
}}

function runSlideshowTick(){{
  clearInterval(slideshowInterval);
  lbBar.style.transition='none';
  lbBar.style.width='0%';
  requestAnimationFrame(()=>{{
    requestAnimationFrame(()=>{{
      lbBar.style.transition='width '+slideshowDelay+'ms linear';
      lbBar.style.width='100%';
    }});
  }});
  slideshowInterval=setTimeout(()=>{{
    if(slideshowRandom)showPhoto(Math.floor(Math.random()*filtered.length));
    else showPhoto(currentIdx+1);
    runSlideshowTick();
  }},slideshowDelay);
}}

function resetSlideshowTimer(){{
  if(slideshowInterval)runSlideshowTick();
}}

function stopSlideshow(){{
  clearInterval(slideshowInterval);
  slideshowInterval=null;
  lbBar.style.width='0%';
  document.querySelector('.slideshow-controls').style.display='none';
  document.getElementById('ss-playpause').textContent='Pause';
}}

document.getElementById('btn-slideshow').addEventListener('click',()=>startSlideshow(false));
document.getElementById('btn-random').addEventListener('click',()=>startSlideshow(true));

document.getElementById('ss-playpause').addEventListener('click',()=>{{
  const btn=document.getElementById('ss-playpause');
  if(slideshowInterval){{clearInterval(slideshowInterval);slideshowInterval=null;lbBar.style.transition='none';btn.textContent='Reprendre';}}
  else{{btn.textContent='Pause';runSlideshowTick();}}
}});

document.getElementById('ss-random-toggle').addEventListener('click',()=>{{
  slideshowRandom=!slideshowRandom;
  document.getElementById('ss-random-toggle').classList.toggle('active',slideshowRandom);
}});

document.getElementById('ss-speed-down').addEventListener('click',()=>{{
  slideshowDelay=Math.min(slideshowDelay+1000,15000);
  document.getElementById('ss-speed').textContent=(slideshowDelay/1000)+'s';
  if(slideshowInterval)runSlideshowTick();
}});

document.getElementById('ss-speed-up').addEventListener('click',()=>{{
  slideshowDelay=Math.max(slideshowDelay-1000,1000);
  document.getElementById('ss-speed').textContent=(slideshowDelay/1000)+'s';
  if(slideshowInterval)runSlideshowTick();
}});

// Init: hide slideshow controls
document.querySelector('.slideshow-controls').style.display='none';
</script>
</body>
</html>"##,
        tags_filter = tags_filter_html,
        grid = grid_html,
        photos_json = photos_json,
    )
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn escape_js(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

pub fn run_gallery(dir: &Path) -> Result<()> {
    let metadata = Metadata::load(dir)?;
    let photos = collect_photos(dir);

    let total: usize = photos.values().map(|v| v.len()).sum();
    if total == 0 {
        anyhow::bail!("Aucune photo trouvée dans {}", dir.display());
    }

    let html = generate_html(&photos, &metadata);
    let output_path = dir.join("gallery.html");
    std::fs::write(&output_path, &html)?;

    println!(
        "{} photos dans la galerie → {}",
        total,
        output_path.display()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn tmpdir() -> std::path::PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "photo_sort_gallery_test_{}_{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setup_photos(dir: &std::path::Path) {
        let y2020 = dir.join("2020");
        let y2021 = dir.join("2021");
        std::fs::create_dir_all(&y2020).unwrap();
        std::fs::create_dir_all(&y2021).unwrap();
        std::fs::write(y2020.join("2020-01-01_00-00-00.jpg"), "fake").unwrap();
        std::fs::write(y2020.join("2020-06-15_12-00-00.jpg"), "fake").unwrap();
        std::fs::write(y2021.join("2021-03-10_09-00-00.jpg"), "fake").unwrap();
    }

    // --- collect_photos ---

    #[test]
    fn collect_photos_finds_by_year() {
        let tmp = tmpdir();
        setup_photos(&tmp);

        let photos = collect_photos(&tmp);
        assert_eq!(photos.len(), 2);
        assert_eq!(photos["2020"].len(), 2);
        assert_eq!(photos["2021"].len(), 1);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn collect_photos_ignores_non_photo_files() {
        let tmp = tmpdir();
        let y = tmp.join("2020");
        std::fs::create_dir_all(&y).unwrap();
        std::fs::write(y.join("readme.txt"), "text").unwrap();
        std::fs::write(y.join("photo.jpg"), "img").unwrap();

        let photos = collect_photos(&tmp);
        assert_eq!(photos["2020"].len(), 1);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn collect_photos_empty_dir() {
        let tmp = tmpdir();
        let photos = collect_photos(&tmp);
        assert!(photos.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn collect_photos_sorted_within_year() {
        let tmp = tmpdir();
        let y = tmp.join("2020");
        std::fs::create_dir_all(&y).unwrap();
        std::fs::write(y.join("b.jpg"), "b").unwrap();
        std::fs::write(y.join("a.jpg"), "a").unwrap();

        let photos = collect_photos(&tmp);
        assert_eq!(photos["2020"], vec!["2020/a.jpg", "2020/b.jpg"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // --- generate_html ---

    #[test]
    fn html_contains_doctype_and_structure() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let photos = collect_photos(&tmp);
        let meta = Metadata::default();
        let html = generate_html(&photos, &meta);

        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<title>photo-sort gallery</title>"));
        assert!(html.contains("lightbox"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn html_contains_all_photos() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let photos = collect_photos(&tmp);
        let meta = Metadata::default();
        let html = generate_html(&photos, &meta);

        assert!(html.contains("2020-01-01_00-00-00.jpg"));
        assert!(html.contains("2020-06-15_12-00-00.jpg"));
        assert!(html.contains("2021-03-10_09-00-00.jpg"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn html_contains_year_headers() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let photos = collect_photos(&tmp);
        let meta = Metadata::default();
        let html = generate_html(&photos, &meta);

        assert!(html.contains("data-year=\"2020\""));
        assert!(html.contains("data-year=\"2021\""));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn html_contains_slideshow_controls() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let photos = collect_photos(&tmp);
        let meta = Metadata::default();
        let html = generate_html(&photos, &meta);

        assert!(html.contains("btn-slideshow"));
        assert!(html.contains("btn-random"));
        assert!(html.contains("ss-playpause"));
        assert!(html.contains("ss-random-toggle"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn html_contains_tags_when_metadata_has_tags() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let photos = collect_photos(&tmp);
        let mut meta = Metadata::default();
        meta.add_tag("2020/2020-01-01_00-00-00.jpg", "vacances");
        meta.add_tag("2020/2020-01-01_00-00-00.jpg", "plage");

        let html = generate_html(&photos, &meta);
        assert!(html.contains("\"vacances\""));
        assert!(html.contains("\"plage\""));
        // Tag filter buttons
        assert!(html.contains("data-tag=\"vacances\""));
        assert!(html.contains("data-tag=\"plage\""));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn html_contains_rating_in_data() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let photos = collect_photos(&tmp);
        let mut meta = Metadata::default();
        meta.set_rating("2020/2020-01-01_00-00-00.jpg", Some(4));

        let html = generate_html(&photos, &meta);
        assert!(html.contains("\"rating\":4"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn html_has_keyboard_navigation() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let photos = collect_photos(&tmp);
        let meta = Metadata::default();
        let html = generate_html(&photos, &meta);

        assert!(html.contains("ArrowLeft"));
        assert!(html.contains("ArrowRight"));
        assert!(html.contains("Escape"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn html_has_rating_filter() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let photos = collect_photos(&tmp);
        let meta = Metadata::default();
        let html = generate_html(&photos, &meta);

        assert!(html.contains("rating-filter"));
        assert!(html.contains("data-rating=\"5\""));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // --- run_gallery ---

    #[test]
    fn run_gallery_creates_html_file() {
        let tmp = tmpdir();
        setup_photos(&tmp);

        run_gallery(&tmp).unwrap();
        assert!(tmp.join("gallery.html").exists());

        let content = std::fs::read_to_string(tmp.join("gallery.html")).unwrap();
        assert!(content.contains("<!DOCTYPE html>"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn run_gallery_empty_dir_errors() {
        let tmp = tmpdir();
        assert!(run_gallery(&tmp).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
