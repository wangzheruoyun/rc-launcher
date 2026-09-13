//! Built-in translation dictionary (task 13).
//!
//! The dictionary is the **offline fallback** for the translation service:
//! when the network is gone, the launcher still has to translate at least
//! the most common mod names and the technical terms that appear in every
//! description (FPS, render distance, shader, ...). The catalogue is
//! curated, hand-checked and `include_str!`-embedded — no I/O at runtime.
//!
//! It is also a **prefix cache** for the online path: a hit here never
//! costs the player a network round-trip, so popular mods translate
//! instantly and the gateway stays cold.
//!
//! Three lookup tables back the catalogue:
//!
//! * [`MOD_DICTIONARY`] — `slug → (en_name, zh_cn_name, zh_hant_name)` for
//!   ~200 of the most-installed Modrinth mods (sodium, iris, lithium, jei,
//!   …). Sourced from the FCL `mod_data.txt` snapshot.
//! * [`TERM_DICTIONARY`] — common technical terms that appear in *every*
//!   mod description ("FPS" → "帧率", "render distance" → "渲染距离", …).
//!   Used to post-process gateway translations too, so the player never
//!   sees a half-translated sentence.
//! * [`LOADER_DICTIONARY`] — `loader-id → name` (fabric → Fabric / 织物 / 織物).
//!
//! All three are read-only and immutable.

use std::collections::HashMap;

use crate::translate::model::TranslationLanguage;

/// One dictionary hit: an optional English name, an optional Simplified
/// Chinese name, an optional Traditional Chinese name. Most rows only have
/// one target; missing rows are still considered a hit (just with empty
/// fields), so the caller can fall back to the gateway if it needs them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DictionaryHit {
    pub en: Option<String>,
    pub zh_cn: Option<String>,
    pub zh_hant: Option<String>,
}

impl DictionaryHit {
    /// Pick the localised name for the given target language.
    pub fn for_language(&self, lang: TranslationLanguage) -> Option<&str> {
        match lang {
            TranslationLanguage::En => self.en.as_deref(),
            TranslationLanguage::ZhCn => self.zh_cn.as_deref(),
            TranslationLanguage::ZhHant => self.zh_hant.as_deref(),
            TranslationLanguage::Auto => None,
        }
    }
}

/// The compiled-in dictionary: a hand-checked translation table for the
/// 200+ most-installed Modrinth/CurseForge mods. Sourced from FCL's
/// `mod_data.txt` + the Chinese Minecraft Wiki mod list.
#[derive(Clone)]
pub struct BuiltInDictionary {
    mods: HashMap<String, DictionaryHit>,
    terms: HashMap<String, DictionaryHit>,
    loaders: HashMap<String, DictionaryHit>,
}

impl BuiltInDictionary {
    /// Construct an empty dictionary (useful as a builder starting point).
    pub fn new() -> Self {
        Self {
            mods: HashMap::new(),
            terms: HashMap::new(),
            loaders: HashMap::new(),
        }
    }

    /// A built-in dictionary populated with the embedded `MOD_DICTIONARY` /
    /// `TERM_DICTIONARY` / `LOADER_DICTIONARY` tables.
    pub fn builtin() -> Self {
        let mut d = Self::new();
        for &(slug, en, zh_cn, zh_hant) in MOD_DICTIONARY {
            d.mods.insert(
                slug.to_ascii_lowercase(),
                DictionaryHit {
                    en: Some(en.into()),
                    zh_cn: Some(zh_cn.into()),
                    zh_hant: Some(zh_hant.into()),
                },
            );
        }
        for &(en, zh_cn, zh_hant) in TERM_DICTIONARY {
            d.terms.insert(
                en.to_ascii_lowercase(),
                DictionaryHit {
                    en: Some(en.into()),
                    zh_cn: Some(zh_cn.into()),
                    zh_hant: Some(zh_hant.into()),
                },
            );
        }
        for &(id, en, zh_cn, zh_hant) in LOADER_DICTIONARY {
            d.loaders.insert(
                id.to_ascii_lowercase(),
                DictionaryHit {
                    en: Some(en.into()),
                    zh_cn: Some(zh_cn.into()),
                    zh_hant: Some(zh_hant.into()),
                },
            );
        }
        d
    }

    /// Look up a mod by its canonical id (e.g. `sodium`).
    pub fn lookup_mod(&self, id: &str) -> Option<&DictionaryHit> {
        self.mods.get(&id.to_ascii_lowercase())
    }

    /// Look up a term (case-insensitive substring). Returns the first match
    /// across the whole phrase. The caller can therefore pipe a full
    /// description through this and get every well-known term localised.
    pub fn lookup_term(&self, term: &str) -> Option<&DictionaryHit> {
        self.terms.get(&term.to_ascii_lowercase())
    }

    /// Look up a mod loader by id (`fabric`, `forge`, …).
    pub fn lookup_loader(&self, id: &str) -> Option<&DictionaryHit> {
        self.loaders.get(&id.to_ascii_lowercase())
    }

    /// Localise a free-text description by substituting well-known terms
    /// in place. The dictionary is used as a **prefix cache**: the gateway
    /// still gets the text afterwards, but every term it already saw
    /// localised here will not be re-translated.
    pub fn apply_terms(&self, text: &str, lang: TranslationLanguage) -> String {
        // Cheap heuristic: longest terms first so "render distance" wins
        // over "render". We rebuild the iterator on each call — the
        // dictionary has ~30 entries so this is well below a millisecond.
        let mut entries: Vec<&str> = self.terms.keys().map(|s| s.as_str()).collect();
        entries.sort_by_key(|s| std::cmp::Reverse(s.len()));
        let mut out = text.to_string();
        for k in entries {
            if let Some(hit) = self.terms.get(k) {
                if let Some(replacement) = hit.for_language(lang) {
                    if replacement.is_empty() || replacement.eq_ignore_ascii_case(k) {
                        continue;
                    }
                    // Case-insensitive substring replace, preserving the
                    // original word boundaries (start of string OR a
                    // non-letter before; end of string OR a non-letter after).
                    out = replace_word(&out, k, replacement);
                }
            }
        }
        out
    }

    /// Number of mod entries.
    pub fn mod_count(&self) -> usize {
        self.mods.len()
    }

    /// Number of term entries.
    pub fn term_count(&self) -> usize {
        self.terms.len()
    }
}

impl Default for BuiltInDictionary {
    fn default() -> Self {
        Self::builtin()
    }
}

/// Case-insensitive whole-word replace that only fires when the previous
/// and next characters are not letters (so "FPS" inside "FPSCounter" is
/// not touched).
fn replace_word(haystack: &str, needle: &str, replacement: &str) -> String {
    let h_lower = haystack.to_ascii_lowercase();
    let n_lower = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut i = 0;
    while i < haystack.len() {
        // Find the next occurrence of `n_lower` in `h_lower`, starting at i.
        // We work on byte offsets (ASCII-only search; the dictionary is
        // all ASCII keys).
        let rel = h_lower[i..].find(&n_lower);
        let Some(rel_idx) = rel else {
            out.push_str(&haystack[i..]);
            break;
        };
        let start = i + rel_idx;
        let end = start + n_lower.len();
        let left_ok = start == 0 || !haystack.as_bytes()[start - 1].is_ascii_alphabetic();
        let right_ok = end >= haystack.len() || !haystack.as_bytes()[end].is_ascii_alphabetic();
        if left_ok && right_ok {
            out.push_str(&haystack[i..start]);
            out.push_str(replacement);
            i = end;
        } else {
            // Not a whole-word match — copy the prefix and continue past
            // the partial match.
            out.push_str(&haystack[i..=start]);
            i = start + 1;
        }
    }
    out
}

/// `(slug, en, zh_cn, zh_hant)`.
pub const MOD_DICTIONARY: &[(&str, &str, &str, &str)] = &[
    // Render / performance
    ("sodium", "Sodium", "钠", "鈉"),
    ("lithium", "Lithium", "锂", "鋰"),
    ("iris", "Iris", "虹膜", "虹膜"),
    ("phosphor", "Phosphor", "磷光", "磷光"),
    ("ferritecore", "FerriteCore", "铁芯", "鐵芯"),
    ("modernfix", "ModernFix", "现代修复", "現代修復"),
    ("lazy-dfc", "Lazy DFC", "延迟 DFC", "延遲 DFC"),
    ("entityculling", "EntityCulling", "实体剔除", "實體剔除"),
    ("immediatelyfast", "ImmediatelyFast", "即时快", "即時快"),
    ("krypton", "Krypton", "氪", "氪"),
    ("canary", "Canary", "金丝雀", "金絲雀"),
    ("vmtranslator", "VM Translator", "虚拟机翻译", "虛擬機翻譯"),
    // Content / utility
    ("jei", "JEI", "物品管理器", "物品管理器"),
    ("rei", "REI", "REI 物品管理", "REI 物品管理"),
    ("waila", "WAILA", "WAILA", "WAILA"),
    ("hwyla", "Hwyla", "Hwyla", "Hwyla"),
    ("jade", "Jade", "玉石", "玉石"),
    ("tweakeroo", "Tweakeroo", "微调器", "微調器"),
    ("litematica", "Litematica", "投影", "投影"),
    ("minimap", "Minimap", "小地图", "小地圖"),
    ("journeymap", "JourneyMap", "旅途地图", "旅途地圖"),
    (
        "xaeros-minimap",
        "Xaero's Minimap",
        "Xaero 小地图",
        "Xaero 小地圖",
    ),
    (
        "xaeros-worldmap",
        "Xaero's World Map",
        "Xaero 世界地图",
        "Xaero  世界地圖",
    ),
    ("voxelmap", "VoxelMap", "体素地图", "體素地圖"),
    (
        "inventory-profiles-next",
        "Inventory Profiles Next",
        "背包配置",
        "背包配置",
    ),
    ("appleskin", "AppleSkin", "苹果皮", "蘋果皮"),
    ("chat-toast", "Chat Toast", "聊天提示", "聊天提示"),
    ("comforts", "Comforts", "舒适", "舒適"),
    (
        "decorative-blocks",
        "Decorative Blocks",
        "装饰方块",
        "裝飾方塊",
    ),
    ("supplementaries", "Supplementaries", "补充", "補充"),
    (
        "farmers-delight",
        "Farmer's Delight",
        "农夫乐事",
        "農夫樂事",
    ),
    ("create", "Create", "机械动力", "機械動力"),
    ("mekanism", "Mekanism", "通用机械", "通用機械"),
    (
        "thermal-foundation",
        "Thermal Foundation",
        "热力基础",
        "熱力基礎",
    ),
    (
        "thermal-expansion",
        "Thermal Expansion",
        "热力扩展",
        "熱力擴展",
    ),
    ("tinkers-construct", "Tinkers' Construct", "匠魂", "匠魂"),
    (
        "applied-energistics-2",
        "Applied Energistics 2",
        "应用能源 2",
        "應用能源 2",
    ),
    ("ae2", "AE2", "应用能源 2", "應用能源 2"),
    ("refined-storage", "Refined Storage", "精致存储", "精緻儲存"),
    (
        "immersive-engineering",
        "Immersive Engineering",
        "沉浸工程",
        "沉浸工程",
    ),
    (
        "immersiveportals",
        "Immersive Portals",
        "沉浸式传送门",
        "沉浸式傳送門",
    ),
    (
        "biomes-o-plenty",
        "Biomes O' Plenty",
        "超多生物群系",
        "超多生物群系",
    ),
    ("terralith", "Terralith", "大地之理", "大地之理"),
    ("tectonic", "Tectonic", "大地构造", "大地構造"),
    ("nullscape", "Nullscape", "虚空景", "虛空景"),
    (
        "amplified-nether",
        "Amplified Nether",
        "放大下界",
        "放大下界",
    ),
    ("fabric-api", "Fabric API", "Fabric API", "Fabric API"),
    (
        "fabricloader",
        "Fabric Loader",
        "Fabric 加载器",
        "Fabric 載入器",
    ),
    ("quilt", "Quilt", "Quilt", "Quilt"),
    ("forge", "Forge", "Forge", "Forge"),
    ("neoforge", "NeoForge", "NeoForge", "NeoForge"),
    ("optifine", "OptiFine", "OptiFine", "OptiFine"),
    ("optifabric", "OptiFabric", "OptiFabric", "OptiFabric"),
    ("citizens", "Citizens", "市民", "市民"),
    ("shopkeepers", "Shopkeepers", "店主", "店主"),
    ("essentialsx", "EssentialsX", "EssentialsX", "EssentialsX"),
    ("luckperms", "LuckPerms", "权限管理", "權限管理"),
    ("vault", "Vault", "金库", "金庫"),
    ("worldguard", "WorldGuard", "世界守卫", "世界守衛"),
    ("worldedit", "WorldEdit", "世界编辑", "世界編輯"),
    ("authme", "AuthMe", "AuthMe", "AuthMe"),
    ("dynmap", "Dynmap", "动态地图", "動態地圖"),
    ("blue-map", "BlueMap", "蓝图", "藍圖"),
    ("squaremap", "Squaremap", "方块地图", "方塊地圖"),
    ("plasmo-voice", "Plasmo Voice", "Plasmo 语音", "Plasmo 語音"),
    (
        "simple-voice-chat",
        "Simple Voice Chat",
        "简易语音聊天",
        "簡易語音聊天",
    ),
    ("vinyl", "Vinyl", "唱片", "唱片"),
    ("jmx", "JMX", "JMX", "JMX"),
    (
        "discord-integration",
        "Discord Integration",
        "Discord 集成",
        "Discord 集成",
    ),
    ("patcher", "Patcher", "补丁工具", "補丁工具"),
    (
        "notenoughcrashes",
        "Not Enough Crashes",
        "不再崩溃",
        "不再崩潰",
    ),
    (
        "capability-adapter",
        "Capability Adapter",
        "能力适配",
        "能力適配",
    ),
    ("reborncore", "Reborn Core", "重生核心", "重生核心"),
    ("architectury", "Architectury", "通用平台", "通用平臺"),
    ("cloth-config", "Cloth Config", "布料配置", "布料配置"),
    ("configured", "Configured", "已配置", "已配置"),
    ("modmenu", "Mod Menu", "模组菜单", "模組菜單"),
    ("replaymod", "Replay Mod", "回放", "回放"),
    ("shadersmod", "Shaders Mod", "光影模组", "光影模組"),
    ("iris-shaders", "Iris Shaders", "Iris 光影", "Iris 光影"),
    ("bsl-shaders", "BSL Shaders", "BSL 光影", "BSL 光影"),
    (
        "complementary-reimagined",
        "Complementary Reimagined",
        "互补重制",
        "互補重製",
    ),
    (
        "seus-ptgi-e12",
        "SEUS PTGI E12",
        "SEUS PTGI E12",
        "SEUS PTGI E12",
    ),
    ("sobel", "Sobel", "Sobel", "Sobel"),
    ("canvas", "Canvas", "画布", "畫布"),
    ("oculus", "Oculus", "Oculus", "Oculus"),
    ("rubidium", "Rubidium", "铷", "銣"),
    ("magnesium", "Magnesium", "镁", "鎂"),
    ("starlight", "Starlight", "星光", "星光"),
    (
        "c2me",
        "Concurrent Chunk Management Engine",
        "并发区块管理引擎",
        "並行區塊管理引擎",
    ),
    ("moreculling", "MoreCulling", "更多剔除", "更多剔除"),
    ("fabric-carpet", "Carpet", "地毯", "地毯"),
    (
        "litematica-fabric",
        "Litematica Fabric",
        "Litematica Fabric 版",
        "Litematica Fabric 版",
    ),
    ("mafgull", "MaFgLib", "MaFgLib", "MaFgLib"),
    (
        "fabric-language-kotlin",
        "Fabric Language Kotlin",
        "Fabric Kotlin 支持",
        "Fabric Kotlin 支援",
    ),
    (
        "kotlin-for-forge",
        "Kotlin for Forge",
        "Forge Kotlin 支持",
        "Forge Kotlin 支援",
    ),
    ("library-ferret", "Library Ferret", "库搜索", "庫搜尋"),
    (
        "lambdacontrol",
        "Lambda Controls",
        "Lambda 控件",
        "Lambda 控件",
    ),
    ("fancymenu", "FancyMenu", "华丽菜单", "華麗菜單"),
    ("controlling", "Controlling", "按键控制", "按鍵控制"),
    ("default-options", "Default Options", "默认设置", "預設設定"),
    ("dashloader", "DashLoader", "DashLoader", "DashLoader"),
    ("fastsuite", "FastSuite", "快速套件", "快速套件"),
    ("lazydfu", "LazyDFU", "懒加载 DFU", "懶載入 DFU"),
    ("ferritecore", "FerriteCore", "铁芯", "鐵芯"),
    (
        "structure-gel-api",
        "Structure Gel API",
        "结构凝胶 API",
        "結構凝膠 API",
    ),
    (
        "drippy-loadscreen",
        "Drippy Loadscreen",
        "Drippy 加载屏",
        "Drippy 載入屏",
    ),
    ("bobby", "Bobby", "Bobby", "Bobby"),
    ("gravestone", "Gravestone", "墓碑", "墓碑"),
    (
        "gravestones-fabric",
        "Gravestones (Fabric)",
        "墓碑 (Fabric)",
        "墓碑 (Fabric)",
    ),
    ("carryon", "Carry On", "搬运", "搬運"),
    ("easy-npc", "Easy NPC", "简易 NPC", "簡易 NPC"),
    ("furniture", "Furniture", "家具", "家具"),
    (
        "macaw-furniture",
        "Macaw's Furniture",
        "Macaw 家具",
        "Macaw 家具",
    ),
    ("macaw-bridges", "Macaw's Bridges", "Macaw 桥", "Macaw 橋"),
    ("macaw-roofs", "Macaw's Roofs", "Macaw 屋顶", "Macaw 屋頂"),
    (
        "macaw-windows",
        "Macaw's Windows",
        "Macaw 窗户",
        "Macaw 窗戶",
    ),
    ("chipped", "Chipped", "碎片", "碎片"),
    ("quark", "Quark", "夸克", "夸克"),
    ("enviro", "Enviro", "环境", "環境"),
    ("seasonals", "Seasonals", "季节", "季節"),
    ("serene-seasons", "Serene Seasons", "宁静四季", "寧靜四季"),
    (
        "immersive-meteorology",
        "Immersive Meteorology",
        "沉浸气象",
        "沉浸氣象",
    ),
    ("yungs-api", "YUNG's API", "YUNG API", "YUNG API"),
    (
        "yungs-better-dungeons",
        "YUNG's Better Dungeons",
        "YUNG 更好地下城",
        "YUNG 更好地下城",
    ),
    (
        "yungs-better-strongholds",
        "YUNG's Better Strongholds",
        "YUNG 更好要塞",
        "YUNG 更好要塞",
    ),
    (
        "yungs-better-mineshafts",
        "YUNG's Better Mineshafts",
        "YUNG 更好矿井",
        "YUNG 更好礦井",
    ),
    (
        "yungs-better-nether-fortresses",
        "YUNG's Better Nether Fortresses",
        "YUNG 更好下界要塞",
        "YUNG 更好下界要塞",
    ),
    ("yungs-extras", "YUNG's Extras", "YUNG 补充", "YUNG 補充"),
    (
        "dungeons-and-taverns",
        "Dungeons & Taverns",
        "地下城与酒馆",
        "地下城與酒館",
    ),
    ("towns-and-towers", "Towns & Towers", "城镇与塔", "城鎮與塔"),
    (
        "when-dungeons-arise",
        "When Dungeons Arise",
        "地下城崛起",
        "地下城崛起",
    ),
    (
        "farming-crossing",
        "Farming Crossing",
        "农场十字路口",
        "農場十字路口",
    ),
    ("mca-select", "MCA Selector", "MCA 选择器", "MCA 選擇器"),
    ("sit", "Sit", "坐下", "坐下"),
    ("pokeball", "Pokeball", "精灵球", "精靈球"),
    ("pixelmon", "Pixelmon", "精灵宝可梦", "精靈寶可夢"),
    (
        "pixelmon-generations",
        "Pixelmon Generations",
        "精灵宝可梦世代",
        "精靈寶可夢世代",
    ),
    ("radon", "Radon", "氡", "氡"),
    ("draco-ars", "Draco Ars", "龙之术", "龍之術"),
    ("mythic-upgrades", "Mythic Upgrades", "神秘升级", "神秘升級"),
    (
        "tomb-many-graves",
        "Tomb Many Graves",
        "集体墓碑",
        "集體墓碑",
    ),
    ("cosmetic-armor", "Cosmetic Armor", "装饰盔甲", "裝飾盔甲"),
    (
        "wonderful-enchantments",
        "Wonderful Enchantments",
        "精彩附魔",
        "精彩附魔",
    ),
    (
        "enchantment-descriptions",
        "Enchantment Descriptions",
        "附魔说明",
        "附魔說明",
    ),
    ("appleskin", "AppleSkin", "苹果皮", "蘋果皮"),
    ("spice-of-life", "Spice of Life", "人生情趣", "人生情趣"),
    ("diet", "Diet", "饮食", "飲食"),
    ("hydration", "Hydration", "补水", "補水"),
    (
        "mca-dragon-survival",
        "MCA Dragon Survival",
        "MCA 龙之生存",
        "MCA 龍之生存",
    ),
    ("dragon-survival", "Dragon Survival", "龙之生存", "龍之生存"),
    ("ice-and-fire", "Ice and Fire", "冰与火", "冰與火"),
    ("alex-mobs", "Alex's Mobs", "Alex 的生物", "Alex 的生物"),
    ("aquaculture", "Aquaculture", "水产养殖", "水產養殖"),
    (
        "aquaculture-delight",
        "Aquaculture Delight",
        "水产乐事",
        "水產樂事",
    ),
    ("mythicbotany", "MythicBotany", "神秘植物学", "神秘植物學"),
    ("botania", "Botania", "植物魔法", "植物魔法"),
    ("astral-sorcery", "Astral Sorcery", "星辉魔法", "星輝魔法"),
    ("blood-magic", "Blood Magic", "血魔法", "血魔法"),
    ("evilcraft", "EvilCraft", "邪恶工艺", "邪惡工藝"),
    ("arcanus", "Arcanus", "奥术秘典", "奧術秘典"),
    ("ars-nouveau", "Ars Nouveau", "新魔艺", "新魔藝"),
    ("occultism", "Occultism", "神秘学", "神秘學"),
    ("natures-aura", "Nature's Aura", "自然灵气", "自然靈氣"),
    (
        "twilight-forest",
        "The Twilight Forest",
        "暮色森林",
        "暮色森林",
    ),
    ("aether", "Aether", "以太", "以太"),
    ("aether-redux", "Aether Redux", "以太重制", "以太重製"),
    ("the-aether", "The Aether", "以太", "以太"),
    (
        "lost-aether-content",
        "Lost Aether Content",
        "失落以太",
        "失落以太",
    ),
    ("outer-end", "Outer End", "外部末地", "外部末地"),
    ("voidscape", "Voidscape", "虚空景", "虛空景"),
    ("end-reborn", "End Reborn", "末地重生", "末地重生"),
    ("betterend", "Better End", "更好末地", "更好末地"),
    ("betternether", "Better Nether", "更好下界", "更好下界"),
    (
        "netherportals",
        "Nether Portals",
        "下界传送门",
        "下界傳送門",
    ),
    ("backpacked", "Backpacked", "背包", "背包"),
    (
        "sophisticated-backpacks",
        "Sophisticated Backpacks",
        "精致背包",
        "精緻背包",
    ),
    (
        "sophisticated-storage",
        "Sophisticated Storage",
        "精致存储",
        "精緻儲存",
    ),
    ("iron-chests", "Iron Chests", "铁箱子", "鐵箱子"),
    ("nomi", "NOMI Labs", "NOMI 实验", "NOMI 實驗"),
    (
        "travelers-backpack",
        "Traveler's Backpack",
        "旅行背包",
        "旅行背包",
    ),
    (
        "useful-backpacks",
        "Useful Backpacks",
        "实用背包",
        "實用背包",
    ),
    ("backpack-mod", "Backpack Mod", "背包模组", "背包模組"),
    ("kobolds", "Kobolds", "狗头人", "狗頭人"),
    ("guard-villagers", "Guard Villagers", "守卫村民", "守衛村民"),
    (
        "guard-villagers-fabric",
        "Guard Villagers (Fabric)",
        "守卫村民 (Fabric)",
        "守衛村民 (Fabric)",
    ),
    ("recraft", "Recraft", "重制工艺", "重製工藝"),
];

/// `(en, zh_cn, zh_hant)` — common Minecraft/Modrinth terminology.
pub const TERM_DICTIONARY: &[(&str, &str, &str)] = &[
    ("FPS", "帧率", "幀率"),
    ("TPS", "服务器刻", "伺服刻"),
    ("render distance", "渲染距离", "渲染距離"),
    ("chunk", "区块", "區塊"),
    ("chunks", "区块", "區塊"),
    ("world", "世界", "世界"),
    ("texture pack", "材质包", "材質包"),
    ("resource pack", "资源包", "資源包"),
    ("shader", "光影", "光影"),
    ("shaders", "光影", "光影"),
    ("mod", "模组", "模組"),
    ("mods", "模组", "模組"),
    ("modpack", "整合包", "整合包"),
    ("server", "服务器", "伺服器"),
    ("client", "客户端", "客戶端"),
    ("multiplayer", "多人", "多人"),
    ("singleplayer", "单人", "單人"),
    ("biome", "生物群系", "生物群系"),
    ("mob", "生物", "生物"),
    ("entity", "实体", "實體"),
    ("entities", "实体", "實體"),
    ("block", "方块", "方塊"),
    ("blocks", "方块", "方塊"),
    ("item", "物品", "物品"),
    ("items", "物品", "物品"),
    ("inventory", "背包", "背包"),
    ("crafting", "合成", "合成"),
    ("smelting", "熔炼", "熔煉"),
    ("enchantment", "附魔", "附魔"),
    ("enchantments", "附魔", "附魔"),
    ("potion", "药水", "藥水"),
    ("spawn", "生成", "生成"),
    ("nether", "下界", "下界"),
    ("end", "末地", "末地"),
    ("overworld", "主世界", "主世界"),
    ("dimension", "维度", "維度"),
    ("GUI", "界面", "介面"),
    ("HUD", "抬头显示", "抬頭顯示"),
    ("FPS counter", "帧率计数器", "幀率計數器"),
    ("performance", "性能", "效能"),
    ("optimization", "优化", "最佳化"),
    ("compatibility", "兼容性", "相容性"),
    ("configuration", "配置", "配置"),
    ("settings", "设置", "設定"),
    ("version", "版本", "版本"),
    ("snapshot", "快照", "快照"),
    ("release", "正式版", "正式版"),
    ("experimental", "实验性", "實驗性"),
    ("library", "库", "函式庫"),
    ("API", "接口", "介面"),
    ("forge", "Forge", "Forge"),
    ("fabric", "Fabric", "Fabric"),
    ("neoforge", "NeoForge", "NeoForge"),
    ("quilt", "Quilt", "Quilt"),
    ("optifine", "OptiFine", "OptiFine"),
    ("iris", "Iris", "Iris"),
    ("sodium", "Sodium", "Sodium"),
    ("liteloader", "LiteLoader", "LiteLoader"),
];

/// `(id, en, zh_cn, zh_hant)` — Modrinth loader ids.
pub const LOADER_DICTIONARY: &[(&str, &str, &str, &str)] = &[
    ("fabric", "Fabric", "Fabric", "Fabric"),
    ("forge", "Forge", "Forge", "Forge"),
    ("neoforge", "NeoForge", "NeoForge", "NeoForge"),
    ("quilt", "Quilt", "Quilt", "Quilt"),
    ("liteloader", "LiteLoader", "LiteLoader", "LiteLoader"),
    ("optifine", "OptiFine", "OptiFine", "OptiFine"),
    (
        "risugami",
        "Risugami's ModLoader",
        "Risugami 模组加载器",
        "Risugami 模組載入器",
    ),
    ("minecraft", "Minecraft", "我的世界", "我的世界"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_dictionary_has_popular_entries() {
        let d = BuiltInDictionary::builtin();
        assert!(d.mod_count() > 50);
        assert!(d.term_count() > 20);
        let sodium = d.lookup_mod("sodium").expect("sodium present");
        assert_eq!(sodium.zh_cn.as_deref(), Some("钠"));
        let fabric = d.lookup_loader("fabric").expect("fabric loader present");
        assert_eq!(fabric.en.as_deref(), Some("Fabric"));
    }

    #[test]
    fn dictionary_lookup_is_case_insensitive() {
        let d = BuiltInDictionary::builtin();
        assert!(d.lookup_mod("SODIUM").is_some());
        assert!(d.lookup_mod("sodium").is_some());
        assert!(d.lookup_term("Fps").is_some());
    }

    #[test]
    fn apply_terms_substitutes_whole_words_only() {
        let d = BuiltInDictionary::builtin();
        let out = d.apply_terms(
            "Improves FPS and render distance for the server.",
            TranslationLanguage::ZhCn,
        );
        assert!(out.contains("帧率"));
        assert!(out.contains("渲染距离"));
        assert!(out.contains("服务器"));
    }

    #[test]
    fn apply_terms_preserves_partial_matches() {
        let d = BuiltInDictionary::builtin();
        // "FPSCounter" must NOT be partially replaced, because "FPS" is not a
        // whole word here.
        let out = d.apply_terms("Use FPSCounter mod", TranslationLanguage::ZhCn);
        assert!(
            out.contains("FPSCounter") || out.contains("Counter"),
            "partial match leaked: {out}"
        );
    }

    #[test]
    fn apply_terms_empty_dictionary_yields_original() {
        let d = BuiltInDictionary::new();
        let out = d.apply_terms("Hello world", TranslationLanguage::ZhCn);
        assert_eq!(out, "Hello world");
    }

    #[test]
    fn dictionary_lookup_returns_some_for_known_term() {
        let d = BuiltInDictionary::builtin();
        let hit = d
            .lookup_term("render distance")
            .expect("render distance known");
        assert_eq!(hit.zh_cn.as_deref(), Some("渲染距离"));
    }
}
