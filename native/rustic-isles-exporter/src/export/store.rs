//! The store runtime capture: view models, item view models and store item images (spec §6.13).

use super::{Ctx, ROW_LIMIT, Row, TextFile, class_name};
use crate::engine::{Engine, sorted_rows};
use crate::json::Json;
use crate::sdk;

pub const VIEW_MODELS: &str = "store_view_models_1_14_7.jsonl";
pub const ITEM_VIEW_MODELS: &str = "store_item_view_models_1_14_7.jsonl";
pub const IMAGES: &str = "store_item_images_1_14_7.jsonl";
const PREFIX: &str = "[StoreRuntime] ";

/// The three files' rows together; -1 when one cannot be opened.
pub fn export(ctx: &Ctx) -> i64 {
    let e = ctx.engine;
    let files = [VIEW_MODELS, ITEM_VIEW_MODELS, IMAGES].map(|name| (name, TextFile::create(&ctx.out.join(name))));
    for (name, file) in &files {
        if file.is_none() {
            ctx.status.line(PREFIX, &format!("cannot open {name}"));
        }
    }
    let [(_, Some(mut views)), (_, Some(mut items)), (_, Some(mut images))] = files else {
        return -1;
    };
    let view_class = e.find_class("StoreViewModel");
    let item_class = e.find_class("StoreItemViewModel");
    let (mut view_count, mut item_count, mut image_count, mut skipped) = (0i64, 0i64, 0i64, 0i64);
    for (_, object) in e.objects() {
        if e.is_a(object, view_class) {
            e.take_fault();
            let mut j = Json::new();
            view_model(&mut j, e, object);
            if e.take_fault() {
                skipped += 1;
            } else {
                views.line(&j.into_bytes());
                view_count += 1;
            }
        }
        if e.is_a(object, item_class) {
            e.take_fault();
            let mut j = Json::new();
            item_view_model(&mut j, e, object);
            if e.take_fault() {
                skipped += 1;
            } else {
                items.line(&j.into_bytes());
                item_count += 1;
            }
        }
    }
    e.take_fault();
    for table in e.tables_by_row_struct("StoreItemTable") {
        let table_name = e.name_of(table);
        let (count, rows) = e.rows(table);
        if !(0..=ROW_LIMIT).contains(&count) {
            ctx.status.line(
                PREFIX,
                &format!("{table_name}: row count {count} out of range, skipping"),
            );
            continue;
        }
        for (row_name, data) in sorted_rows(&rows) {
            e.take_fault();
            let mut j = Json::new();
            j.obj().kstr("table", &table_name).kstr("rowName", &row_name);
            Row { j: &mut j, e, at: data }
                .s("featureImage", &sdk::IMG_FEATURE)
                .s("standardImage", &sdk::IMG_STANDARD);
            j.end_obj();
            if data == 0 || e.take_fault() {
                skipped += 1;
                continue;
            }
            images.line(&j.into_bytes());
            image_count += 1;
        }
    }
    views.finish();
    items.finish();
    images.finish();
    ctx.status.line(
        PREFIX,
        &format!(
            "DONE storeViewModels={view_count} itemViewModels={item_count} imageRows={image_count} skipped={skipped}"
        ),
    );
    view_count + item_count + image_count
}

fn header(j: &mut Json, e: &Engine, object: u64) {
    j.obj()
        .kstr("objectName", &e.name_of(object))
        .kstr("objectClass", &class_name(e, object))
        .kbool("defaultObject", e.is_default(object));
}

fn view_model(j: &mut Json, e: &Engine, object: u64) {
    header(j, e, object);
    Row {
        j: &mut *j,
        e,
        at: object,
    }
    .s("tagFilter", &sdk::SV_TAG_FILTER)
    .s("storeItemsTable", &sdk::SV_ITEMS_TABLE)
    .s("currencyDataTable", &sdk::SV_CURRENCY_TABLE);
    j.key("categories").arr();
    for category in e.items(object, &sdk::SV_CATEGORIES) {
        j.obj();
        Row {
            j: &mut *j,
            e,
            at: category,
        }
        .s("name", &sdk::SV_CAT_NAME)
        .s("breadcrumbCategory", &sdk::SV_CAT_BREADCRUMB)
        .b("requiresEntitlement", &sdk::SV_CAT_REQUIRES)
        .s("entitlementId", &sdk::SV_CAT_ENTITLEMENT)
        .s("skuId", &sdk::SV_CAT_SKU);
        j.key("subcategories").arr();
        for sub in e.items(category, &sdk::SV_CAT_SUBCATEGORIES) {
            j.obj();
            Row { j: &mut *j, e, at: sub }
                .s("name", &sdk::SV_SUB_NAME)
                .s("breadcrumbCategory", &sdk::SV_SUB_BREADCRUMB)
                .b("requiresEntitlement", &sdk::SV_SUB_REQUIRES)
                .s("entitlementId", &sdk::SV_SUB_ENTITLEMENT)
                .s("skuId", &sdk::SV_SUB_SKU)
                .list("subTagIds", &sdk::SV_SUB_TAG_IDS)
                .b("hidePurchasedItems", &sdk::SV_SUB_HIDE_PURCHASED)
                .b("shouldHideTab", &sdk::SV_SUB_SHOULD_HIDE_TAB)
                .b("hideTab", &sdk::SV_SUB_HIDE_TAB);
            j.end_obj();
        }
        j.end_arr().end_obj();
    }
    j.end_arr().end_obj();
}

fn item_view_model(j: &mut Json, e: &Engine, object: u64) {
    header(j, e, object);
    Row {
        j: &mut *j,
        e,
        at: object,
    }
    .s("displayName", &sdk::SI_DISPLAY_NAME)
    .s("displayDescription", &sdk::SI_DISPLAY_DESCRIPTION)
    .s("displayItemType", &sdk::SI_DISPLAY_ITEM_TYPE)
    .n("itemPriceNumber", &sdk::SI_PRICE)
    .n("itemCurrentPriceNumber", &sdk::SI_CURRENT_PRICE)
    .n("storeCurrencyEnum", &sdk::SI_CURRENCY_ENUM)
    .s("currencyCode", &sdk::SI_CURRENCY_CODE)
    .n("displayPriority", &sdk::SI_PRIORITY)
    .b("purchased", &sdk::SI_PURCHASED)
    .b("canPurchaseAgain", &sdk::SI_CAN_PURCHASE_AGAIN)
    .b("expired", &sdk::SI_EXPIRED)
    .b("prestigeItemElite", &sdk::SI_PRESTIGE_ELITE)
    .n("rewardRarityLevel", &sdk::SI_RARITY)
    .s("catalogItemId", &sdk::SI_CATALOG_ITEM_ID);
    let offer = object.wrapping_add(u64::from(sdk::SI_OFFER.offset));
    j.key("offer").obj();
    let mut row = Row {
        j: &mut *j,
        e,
        at: offer,
    };
    row.s("skuId", &sdk::OF_SKU)
        .s("displayName", &sdk::OF_DISPLAY_NAME)
        .s("displayDescription", &sdk::OF_DISPLAY_DESCRIPTION)
        .n("displayPriority", &sdk::OF_PRIORITY)
        .s("catalogId", &sdk::OF_CATALOG_ID)
        .s("imageUrl", &sdk::OF_IMAGE_URL)
        .n("platPrice", &sdk::OF_PLAT_PRICE)
        .n("regularPlatPrice", &sdk::OF_REGULAR_PLAT_PRICE)
        .n("dustPrice", &sdk::OF_DUST_PRICE)
        .n("maxAllowed", &sdk::OF_MAX_ALLOWED)
        .n("remaining", &sdk::OF_REMAINING)
        .list("tags", &sdk::OF_TAGS);
    row.j.key("grantedCatalogIds").arr();
    for granted in e.items(offer, &sdk::OF_GRANTED_IDS) {
        row.j.obj();
        Row {
            j: &mut *row.j,
            e,
            at: granted,
        }
        .s("itemId", &sdk::OF_GRANTED_ID_ITEM)
        .n("amount", &sdk::OF_GRANTED_ID_AMOUNT);
        row.j.end_obj();
    }
    row.j.end_arr();
    row.list("grantedEntitlements", &sdk::OF_GRANTED_ENTITLEMENTS)
        .list("grantedProgressInTracks", &sdk::OF_GRANTED_TRACKS);
    row.j.key("grantedProgress").obj();
    row.s("trackId", &sdk::OF_GRANTED_PROGRESS_TRACK)
        .n("amount", &sdk::OF_GRANTED_PROGRESS_AMOUNT)
        .n("ranks", &sdk::OF_GRANTED_PROGRESS_RANKS);
    row.j.end_obj();
    row.n("grantedLoadoutSlots", &sdk::OF_LOADOUT_SLOTS)
        .b("payWithAllCurrencies", &sdk::OF_ALL_CURRENCIES)
        .s("currencyCode", &sdk::OF_CURRENCY_CODE)
        .n("currencyEnum", &sdk::OF_CURRENCY_ENUM)
        .b("inUserCollection", &sdk::OF_IN_COLLECTION)
        .s("platformOfferId", &sdk::OF_PLATFORM_OFFER_ID)
        .s("timeAvailabilityReason", &sdk::OF_TIME_REASON)
        .list("missingEntitlementNames", &sdk::OF_MISSING_ENTITLEMENTS)
        .list("previewItemIds", &sdk::OF_PREVIEW_IDS);
    j.end_obj().end_obj();
}
