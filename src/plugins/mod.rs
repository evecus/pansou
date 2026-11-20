//! 爬虫插件集合。
//! 原有 20 个插件保留；新增插件依据用户提供的测速结果，只适配实测 OK 的插件。

pub mod helpers;
pub mod common;
pub mod flarum;
pub mod json_simple;
pub mod json_crypto;
pub mod html_blog;
pub mod html_video;
pub mod html_complex;
pub mod apicms;
pub mod json_search;
pub mod html_ok;
pub mod apple_ok;
pub mod json_ok;
pub mod html_ok2;
pub mod html_ok3;
pub mod html_ok4;
pub mod json_ok2;
pub mod new_wave;

use crate::plugin::SearchPlugin;
use crate::plugins::flarum::FlarumPlugin;
use std::sync::Arc;

/// 所有已移植插件
pub fn all() -> Vec<Arc<dyn SearchPlugin>> {
    let mut v: Vec<Arc<dyn SearchPlugin>> = vec![
        // ---- 原有插件（上一阶段成果，保留）----
        Arc::new(json_simple::Jikepan),
        Arc::new(json_simple::Meitizy),
        Arc::new(json_simple::Melost),
        Arc::new(json_simple::Yunsou),
        Arc::new(json_simple::Ash),
        Arc::new(json_crypto::Sdso),
        Arc::new(json_crypto::Yunso),
        Arc::new(FlarumPlugin {
            name: "quark4k",
            base_url: "https://quark4k.com/api/discussions",
            referer: "https://quark4k.com/",
            priority: 3,
            skip_filter: true,
        }),
        Arc::new(html_blog::Kkv),
        Arc::new(html_blog::Alupan),
        Arc::new(html_blog::Xiaoji),
        Arc::new(html_video::Muou),
        Arc::new(html_video::Duoduo),
        Arc::new(html_video::Zxzj),
        Arc::new(html_video::Hdmoli),
        Arc::new(html_video::Quarktv),
        Arc::new(html_complex::Mikuclub),
        Arc::new(html_complex::Gaoqing888),
        Arc::new(html_complex::Panta),
        Arc::new(html_complex::Susu),

        // ---- 新增：测速 OK 插件（依据 speedtest_result.json）----
        Arc::new(json_search::Haisou),
        Arc::new(html_ok::Quarksoo),
        Arc::new(html_ok::Aikanzy),
        Arc::new(html_ok::Xiaozhang),
        Arc::new(html_ok::Pianku),
        Arc::new(apple_ok::Fox4k),
        Arc::new(json_ok::Nyaa),
        Arc::new(json_ok::Pansearch),
        Arc::new(json_ok::Qiwei),
        Arc::new(html_ok2::Mizixing),
        Arc::new(html_ok2::Ypfxw),
        Arc::new(html_ok3::Yulinshufa),
        Arc::new(html_ok3::Panwiki),
        Arc::new(json_ok2::Hunhepan),
        Arc::new(html_ok4::Duanjuw),
        Arc::new(html_ok4::Huban),
        Arc::new(html_ok4::Shandian),
        Arc::new(html_ok4::Jutoushe),
        Arc::new(html_ok4::Xinjuc),
        Arc::new(html_ok4::U3c3),

        // ---- 新版本 pansou-main 独有插件 ----
        Arc::new(new_wave::IkanTv),
        Arc::new(new_wave::QuarkRes),
        Arc::new(new_wave::Aipan),
        Arc::new(new_wave::Haitunsou),
        Arc::new(new_wave::Xiaoyu),
    ];
    // 去重（按 name），防止重复注册
    let mut seen = std::collections::HashSet::new();
    v.retain(|p| seen.insert(p.name().to_string()));
    v
}
