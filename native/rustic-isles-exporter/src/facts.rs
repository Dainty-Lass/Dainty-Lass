//! Every layout fact the exporter takes from the host's SDK (spec §4.5, §9). `build.rs` resolves each one and fails
//! the build naming those it cannot find; the library reads the results as `crate::sdk::<ID>`.
//!
//! The member paths are the spec's (§6); where a host's first build reports one missing, the fix is a spec correction
//! and then the path here (README, "First build").

use crate::sdk_parse::{Kind, Member};

/// `(id, path)`; see `Sdk::resolve` for the path syntax.
pub const MEMBERS: &[(&str, &str)] = &[
    // The engine (spec §4.1)
    ("UOBJECT_FLAGS", "UObject.Flags"),
    ("UOBJECT_CLASS", "UObject.Class"),
    ("UOBJECT_NAME", "UObject.Name"),
    ("UOBJECT_OUTER", "UObject.Outer"),
    ("USTRUCT_SUPER", "UStruct.SuperStruct"),
    ("DATATABLE_ROW_STRUCT", "UDataTable.RowStruct"),
    ("DATATABLE_ROW_MAP", "UDataTable.RowMap"),
    ("TEXT_DATA_SOURCE", "FTextData.TextSource"),
    ("TAG_NAME", "FGameplayTag.TagName"),
    ("TAG_CONTAINER_TAGS", "FGameplayTagContainer.GameplayTags"),
    ("TAG_QUERY_VERSION", "FGameplayTagQuery.TokenStreamVersion"),
    ("TAG_QUERY_DICTIONARY", "FGameplayTagQuery.TagDictionary"),
    ("TAG_QUERY_STREAM", "FGameplayTagQuery.QueryTokenStream"),
    ("TAG_QUERY_USER_DESCRIPTION", "FGameplayTagQuery.UserDescription"),
    ("TAG_QUERY_AUTO_DESCRIPTION", "FGameplayTagQuery.AutoDescription"),
    ("ROW_HANDLE_TABLE", "FDataTableRowHandle.DataTable"),
    ("ROW_HANDLE_ROW", "FDataTableRowHandle.RowName"),
    ("SOFT_PATH_NAME", "FSoftObjectPath.AssetPathName"),
    // GetAllItems (spec §4.2)
    ("GET_ALL_ITEMS_OUT", "ArchonCatalog_GetAllItems.@TArray"),
    // Catalog (spec §6.1)
    ("CAT_ITEM_ID", "FArchonCatalogItem.ItemId"),
    ("CAT_ITEM_CLASS", "FArchonCatalogItem.ItemClass.AssetPathName"),
    ("CAT_DISPLAY_NAME", "FArchonCatalogItem.DisplayNameInvariant"),
    ("CAT_DESCRIPTION", "FArchonCatalogItem.DescriptionInvariant"),
    ("CAT_TAGS", "FArchonCatalogItem.Tags"),
    ("CAT_CUSTOM_DATA", "FArchonCatalogItem.CustomData"),
    ("CAT_ENTITLEMENTS", "FArchonCatalogItem.Entitlements"),
    ("CAT_IS_STACKABLE", "FArchonCatalogItem.IsStackable"),
    ("CAT_IS_BUNDLE", "FArchonCatalogItem.IsBundle"),
    ("CAT_IS_CONTAINER", "FArchonCatalogItem.IsContainer"),
    ("CAT_MAX_QUANTITY", "FArchonCatalogItem.MaxQuantity"),
    ("CAT_CONTAINER_CONTENTS", "FArchonCatalogItem.ContainerItemContents"),
    ("CAT_BUNDLED_ITEMS", "FArchonCatalogItem.BundledItems"),
    (
        "CAT_CONTAINER_CONTENTS_Q",
        "FArchonCatalogItem.ContainerItemContentsWithQuantity",
    ),
    (
        "CAT_CONTAINER_TABLES",
        "FArchonCatalogItem.ContainerResultTableContents",
    ),
    (
        "CAT_CONTAINER_TABLES_Q",
        "FArchonCatalogItem.ContainerResultTableContentsWithQuantity",
    ),
    ("CAT_BUNDLED_ITEMS_Q", "FArchonCatalogItem.BundledItemsWithQuantity"),
    ("CAT_BUNDLED_TABLES", "FArchonCatalogItem.BundledResultTables"),
    (
        "CAT_BUNDLED_TABLES_Q",
        "FArchonCatalogItem.BundledResultTablesWithQuantity",
    ),
    ("CAT_OTHER_QUANTITIES", "FArchonCatalogItem.OtherItemQuantities"),
    ("CAT_ALTERNATE_ITEMS", "FArchonCatalogItem.AlternateItemsToGrant"),
    (
        "CAT_QUANTITY_ITEM",
        "FArchonCatalogItem.ContainerItemContentsWithQuantity[].Item",
    ),
    (
        "CAT_QUANTITY_AMOUNT",
        "FArchonCatalogItem.ContainerItemContentsWithQuantity[].Amount",
    ),
    ("CAT_PRICES", "FArchonCatalogItem.VirtualCurrencyPrices"),
    (
        "CAT_PRICE_CURRENCY",
        "FArchonCatalogItem.VirtualCurrencyPrices[].CurrencyType",
    ),
    ("CAT_PRICE_AMOUNT", "FArchonCatalogItem.VirtualCurrencyPrices[].Amount"),
    (
        "CAT_JOURNEY_UNLOCK_ROW",
        "FArchonCatalogItem.PlayerJourneyUnlockRow.RowName",
    ),
    // Slayer's Path (spec §6.9)
    ("SP_NODE_TYPE", "FPlayerJourneyNodeData.NodeType"),
    ("SP_NODE_LEVEL", "FPlayerJourneyNodeData.NodeLevel"),
    ("SP_CHILD_NODES", "FPlayerJourneyNodeData.ChildNodes"),
    ("SP_AUTO_UNLOCK", "FPlayerJourneyNodeData.bAutoUnlockIfParentUnlocked"),
    ("SP_CURRENCY_COSTS", "FPlayerJourneyNodeData.CurrencyCosts"),
    (
        "SP_CURRENCY_COST_CURRENCY",
        "FPlayerJourneyNodeData.CurrencyCosts[].Currency",
    ),
    (
        "SP_CURRENCY_COST_AMOUNT",
        "FPlayerJourneyNodeData.CurrencyCosts[].Amount",
    ),
    ("SP_EXPERIENCE_COSTS", "FPlayerJourneyNodeData.ExperienceCosts"),
    (
        "SP_EXPERIENCE_COST_TYPE",
        "FPlayerJourneyNodeData.ExperienceCosts[].ExperienceType",
    ),
    (
        "SP_EXPERIENCE_COST_AMOUNT",
        "FPlayerJourneyNodeData.ExperienceCosts[].Amount",
    ),
    ("SP_REWARDS", "FPlayerJourneyNodeData.Rewards"),
    ("SP_REWARD_ITEM_ID", "FPlayerJourneyNodeData.Rewards[].ItemId"),
    ("SP_REWARD_AMOUNT", "FPlayerJourneyNodeData.Rewards[].Amount"),
    (
        "SP_REWARD_ENTITLEMENT_ID",
        "FPlayerJourneyNodeData.Rewards[].EntitlementId",
    ),
    ("SP_REWARD_DURATION", "FPlayerJourneyNodeData.Rewards[].Duration"),
    ("SP_REWARD_PASS_PREFIX", "FPlayerJourneyNodeData.Rewards[].PassPrefix"),
    (
        "SP_REWARD_EXPERIENCE_TYPE",
        "FPlayerJourneyNodeData.Rewards[].ExperienceType",
    ),
    ("SP_REWARD_AUTO_EQUIP", "FPlayerJourneyNodeData.Rewards[].bAutoEquip"),
    ("SP_SYSTEM_REWARDS", "FPlayerJourneyNodeData.SystemRewards"),
    ("SP_ACTIVATION_EVENT_ID", "FPlayerJourneyNodeData.ActivationEventId"),
    ("SP_QUEST_IDS", "FPlayerJourneyNodeData.QuestIds"),
    ("SP_QUEST_TO_NOTIFY", "FPlayerJourneyNodeData.QuestIdToNotifyOnUnlock"),
    ("SP_TUTORIAL", "FPlayerJourneyNodeData.TutorialToShowWhenUnlocked"),
    ("SP_OBJECTIVES", "FPlayerJourneyNodeData.Objectives"),
    ("SP_ATTRIBUTE_VALUES", "FPlayerJourneyNodeData.GameplayAttributeValues"),
    // Hunts (spec §6.10): player hunts
    ("PH_ID", "FPlayerHuntTableData.PlayerHuntId"),
    ("PH_REGION", "FPlayerHuntTableData.Region"),
    ("PH_MATCHMAKING_TYPE", "FPlayerHuntTableData.MatchmakingType"),
    ("PH_MATCHMAKING_GAME_TYPE", "FPlayerHuntTableData.MatchmakingGameType"),
    ("PH_HUNT_TAGS", "FPlayerHuntTableData.HuntTags"),
    ("PH_MATCHMAKER_HUNT_IDS", "FPlayerHuntTableData.MatchmakerHuntIDs"),
    ("PH_BY_TAG", "FPlayerHuntTableData.MatchmakerHuntsByTag"),
    (
        "PH_BY_TAG_TABLE",
        "FPlayerHuntTableData.MatchmakerHuntsByTag[].MatchmakerTable",
    ),
    (
        "PH_BY_TAG_QUERIES",
        "FPlayerHuntTableData.MatchmakerHuntsByTag[].HuntTags",
    ),
    ("PH_ESCALATION_SPEC", "FPlayerHuntTableData.EscalationModeSpecification"),
    (
        "PH_PATROL_INITIAL",
        "FPlayerHuntTableData.EscalationPatrolInitialChallengeLevel",
    ),
    (
        "PH_PATROL_MAX",
        "FPlayerHuntTableData.EscalationPatrolMaxChallengeLevel",
    ),
    (
        "PH_RECOMMENDED_ESCALATION",
        "FPlayerHuntTableData.RecommendedEscalationLevel",
    ),
    ("PH_HAS_PORTALS", "FPlayerHuntTableData.bHasPortals"),
    ("PH_HAS_GLITTER", "FPlayerHuntTableData.bHasGlitterEvent"),
    ("PH_HAS_PHAELANX", "FPlayerHuntTableData.bHasPhaelanxEvent"),
    ("PH_SUCCESS_REWARD", "FPlayerHuntTableData.HuntSuccessReward"),
    ("PH_FIRST_REWARD", "FPlayerHuntTableData.FirstCompletionSuccessReward"),
    (
        "PH_TARGETED_SUCCESS_REWARD",
        "FPlayerHuntTableData.TargetedHuntSuccessReward",
    ),
    ("PH_FAILURE_REWARD", "FPlayerHuntTableData.HuntFailureReward"),
    (
        "PH_TARGETED_FAILURE_REWARD",
        "FPlayerHuntTableData.TargetedHuntFailureReward",
    ),
    ("PH_MIN_SKILL", "FPlayerHuntTableData.MinWeaponSkillLevel"),
    (
        "PH_RECOMMENDED_SKILL",
        "FPlayerHuntTableData.RecomendedWeaponSkillLevel",
    ),
    ("PH_VISIBLE_LOCKED", "FPlayerHuntTableData.IsVisibleWhileLocked"),
    // matchmaker hunts
    ("MH_REGION", "FMatchmakerHuntTableData.Region"),
    ("MH_HUNT_TAGS", "FMatchmakerHuntTableData.HuntTags"),
    ("MH_THREAT", "FMatchmakerHuntTableData.HuntThreatLevel"),
    ("MH_GAME_MODE", "FMatchmakerHuntTableData.GameModeOverride"),
    ("MH_DANGER", "FMatchmakerHuntTableData.DangerPerSecOverride"),
    ("MH_BEHEMOTH", "FMatchmakerHuntTableData.SpecificBehemoth"),
    (
        "MH_BEHEMOTH_NAME",
        "FMatchmakerHuntTableData.SpecificBehemoth.BehemothName",
    ),
    (
        "MH_BEHEMOTH_ASSET",
        "FMatchmakerHuntTableData.SpecificBehemoth.BehemothAsset",
    ),
    (
        "MH_BEHEMOTH_POWER",
        "FMatchmakerHuntTableData.SpecificBehemoth.PowerOverride",
    ),
    (
        "MH_BEHEMOTH_WEIGHT",
        "FMatchmakerHuntTableData.SpecificBehemoth.Weighting",
    ),
    (
        "MH_EXTRA_BEHEMOTHS",
        "FMatchmakerHuntTableData.AdditionalSpecificBehemoths",
    ),
    ("MH_MAP_METADATA", "FMatchmakerHuntTableData.MapMetaData"),
    ("MH_MAP_LIST", "FMatchmakerHuntTableData.MapList"),
    ("MH_MAP_NAME", "FMatchmakerHuntTableData.MapList[].MapName"),
    ("MH_MAP_ASSET_NAME", "FMatchmakerHuntTableData.MapList[].MapAssetName"),
    ("MH_MAP_ASSET", "FMatchmakerHuntTableData.MapList[].MapAsset"),
    ("MH_MAP_BIOME", "FMatchmakerHuntTableData.MapList[].Biome"),
    ("MH_MAP_WEIGHT", "FMatchmakerHuntTableData.MapList[].Weighting"),
    ("MH_SPECIFIC_MODIFIER", "FMatchmakerHuntTableData.SpecificModifier"),
    ("MH_MODIFIERS", "FMatchmakerHuntTableData.Modifiers"),
    ("MH_GATHERABLE_TIER", "FMatchmakerHuntTableData.GatherableTier"),
    ("MH_MAX_PLAYERS", "FMatchmakerHuntTableData.MaxPlayers"),
    ("MH_GENERATED", "FMatchmakerHuntTableData.bIsGeneratedEncounter"),
    ("MH_MODE_DATA", "FMatchmakerHuntTableData.GameModeSpecificData"),
    // hunt regions
    ("HR_MATCHMAKING_ID", "FHunt_Region.MatchmakingId"),
    ("HR_TOKEN_ID", "FHunt_Region.TokenId"),
    ("HR_TOKENS_REQUIRED", "FHunt_Region.NumTokensRequired"),
    ("HR_PASS_REWARDS", "FHunt_Region.NumHuntPassBehemothRewards"),
    ("HR_TARGETED_LIST", "FHunt_Region.TargetedBehemothList"),
    ("HR_PRESETS", "FHunt_Region.BehemothPresets"),
    // escalation specifications
    ("ES_INITIAL", "FEscalationModeSpecification.InitialChallengeLevel"),
    ("ES_MAX", "FEscalationModeSpecification.MaxChallengeLevel"),
    ("ES_ROUNDS", "FEscalationModeSpecification.RoundStructures"),
    ("ES_LOOT_TABLE", "FEscalationModeSpecification.LootTable"),
    (
        "ES_POWER_OVERRIDE",
        "FEscalationModeSpecification.OptionalPowerOverride",
    ),
    ("ES_TAGS", "FEscalationModeSpecification.EscalationModeTags"),
    ("ES_BACKUP_TABLE", "FEscalationModeSpecification.BackupEncountersTable"),
    (
        "ES_COMPLETION_REWARDS",
        "FEscalationModeSpecification.CompletionRewards",
    ),
    (
        "ES_AMOUNT_PER_SCORE",
        "FEscalationModeSpecification.EscalationAmountPerScoreValue",
    ),
    // hunt modifiers
    ("HM_ASSET", "FHuntModifierTableRow.ModifierAsset"),
    ("HM_COMPLEXITY", "FHuntModifierTableRow.HuntComplexityModifier"),
    ("HM_THREAT", "FHuntModifierTableRow.ThreatLevel"),
    ("HM_APPLICATION", "FHuntModifierTableRow.ModifierApplicationType"),
    ("HM_META_TAGS", "FHuntModifierTableRow.ModifierMetaTags"),
    ("HM_ELEMENT", "FHuntModifierTableRow.BehemothElementCondition"),
    // game activities
    ("GA_ENABLED", "FGameActivityTableData.bEnabled"),
    ("GA_PLAYER_HUNT", "FGameActivityTableData.PlayerHunt"),
    ("GA_MATCHMAKER_HUNTS", "FGameActivityTableData.MatchmakerHunts"),
    ("GA_UNLOCK_REQUIREMENT", "FGameActivityTableData.UnlockRequirement"),
    ("GA_UNLOCK_CRITERIA", "FGameActivityTableData.UnlockCriteria"),
    // activity unlock criteria
    ("UC_FLAGS_OP", "FProgressionUnlockCriteria.FeatureFlagsOperation"),
    ("UC_FLAGS", "FProgressionUnlockCriteria.FeatureFlags"),
    ("UC_ITEMS_OP", "FProgressionUnlockCriteria.ItemIdsOperation"),
    ("UC_ITEMS", "FProgressionUnlockCriteria.ItemIds"),
    (
        "UC_RANKS_OP",
        "FProgressionUnlockCriteria.ProgressionTrackRanksOperation",
    ),
    ("UC_RANKS", "FProgressionUnlockCriteria.ProgressionTrackRanks"),
    (
        "UC_RANK_TRACK",
        "FProgressionUnlockCriteria.ProgressionTrackRanks[].Track",
    ),
    (
        "UC_RANK_RANK",
        "FProgressionUnlockCriteria.ProgressionTrackRanks[].Rank",
    ),
    ("UC_QUESTS_OP", "FProgressionUnlockCriteria.QuestRequirementsOperation"),
    (
        "UC_QUESTS_CONDITION",
        "FProgressionUnlockCriteria.QuestRequirementsCondition",
    ),
    ("UC_QUESTS", "FProgressionUnlockCriteria.QuestRequirements"),
    (
        "UC_QUEST_NAME",
        "FProgressionUnlockCriteria.QuestRequirements[].QuestName",
    ),
    (
        "UC_QUEST_STATUS",
        "FProgressionUnlockCriteria.QuestRequirements[].QuestStatusRequired",
    ),
    ("UC_NODES_OP", "FProgressionUnlockCriteria.PlayerJourneyNodesOperation"),
    ("UC_NODES", "FProgressionUnlockCriteria.PlayerJourneyNodes"),
    // schedules
    ("SD_START", "FScheduleData.StartTime"),
    ("SD_END", "FScheduleData.EndTime"),
    ("SD_REPEATABLE", "FScheduleData.IsRepeatable"),
    ("SD_EACH_DAY", "FScheduleData.RepeatableDetails.EachDay"),
    ("SD_EACH_WEEK", "FScheduleData.RepeatableDetails.EachWeek"),
    ("SD_SHOULD_QUEUE", "FScheduleData.bShouldQueueTheSheduleItems"),
    ("SD_ITEMS", "FScheduleData.ScheduledItems"),
    ("SD_ITEM_ID", "FScheduleData.ScheduledItems[].ID"),
    ("SD_ITEM_MAX", "FScheduleData.ScheduledItems[].MaxCompletionPerInterval"),
    ("SD_LEFT_OUT", "FScheduleData.ItemsToBeLeftOutOfTheRotation"),
    // Store runtime (spec §6.13): store view models
    ("SV_ITEMS_TABLE", "UStoreViewModel.StoreItemsTable"),
    ("SV_CURRENCY_TABLE", "UStoreViewModel.CurrencyDataTable"),
    ("SV_CATEGORIES", "UStoreViewModel.StoreCategoryViewModels"),
    ("SV_CAT_NAME", "UStoreViewModel.StoreCategoryViewModels[].CategoryName"),
    (
        "SV_CAT_BREADCRUMB",
        "UStoreViewModel.StoreCategoryViewModels[].BreadcrumbCategory",
    ),
    (
        "SV_CAT_REQUIRES",
        "UStoreViewModel.StoreCategoryViewModels[].RequiresEntitlement",
    ),
    (
        "SV_CAT_ENTITLEMENT",
        "UStoreViewModel.StoreCategoryViewModels[].EntitlementId",
    ),
    ("SV_CAT_SKU", "UStoreViewModel.StoreCategoryViewModels[].SkuId"),
    (
        "SV_CAT_SUBCATEGORIES",
        "UStoreViewModel.StoreCategoryViewModels[].SubCategoryViewModels",
    ),
    (
        "SV_SUB_NAME",
        "UStoreViewModel.StoreCategoryViewModels[].SubCategoryViewModels[].SubCategoryName",
    ),
    (
        "SV_SUB_BREADCRUMB",
        "UStoreViewModel.StoreCategoryViewModels[].SubCategoryViewModels[].BreadcrumbCategory",
    ),
    (
        "SV_SUB_REQUIRES",
        "UStoreViewModel.StoreCategoryViewModels[].SubCategoryViewModels[].RequiresEntitlement",
    ),
    (
        "SV_SUB_ENTITLEMENT",
        "UStoreViewModel.StoreCategoryViewModels[].SubCategoryViewModels[].EntitlementId",
    ),
    (
        "SV_SUB_SKU",
        "UStoreViewModel.StoreCategoryViewModels[].SubCategoryViewModels[].SkuId",
    ),
    (
        "SV_SUB_TAG_IDS",
        "UStoreViewModel.StoreCategoryViewModels[].SubCategoryViewModels[].SubTagIds",
    ),
    (
        "SV_SUB_HIDE_PURCHASED",
        "UStoreViewModel.StoreCategoryViewModels[].SubCategoryViewModels[].HidePurchasedItems",
    ),
    (
        "SV_SUB_SHOULD_HIDE_TAB",
        "UStoreViewModel.StoreCategoryViewModels[].SubCategoryViewModels[].bShouldHideTab",
    ),
    (
        "SV_SUB_HIDE_TAB",
        "UStoreViewModel.StoreCategoryViewModels[].SubCategoryViewModels[].bHideTab",
    ),
    // store item view models
    ("SI_DISPLAY_NAME", "UStoreItemViewModel.DisplayName"),
    ("SI_DISPLAY_DESCRIPTION", "UStoreItemViewModel.DisplayDescription"),
    ("SI_DISPLAY_ITEM_TYPE", "UStoreItemViewModel.DisplayItemType"),
    ("SI_PRICE", "UStoreItemViewModel.ItemPriceNumber"),
    ("SI_CURRENT_PRICE", "UStoreItemViewModel.ItemCurrentPriceNumber"),
    ("SI_CURRENCY_ENUM", "UStoreItemViewModel.StoreCurrency"),
    ("SI_CURRENCY_CODE", "UStoreItemViewModel.CurrencyCode"),
    ("SI_PRIORITY", "UStoreItemViewModel.DisplayPriority"),
    ("SI_PURCHASED", "UStoreItemViewModel.bIsPurchased"),
    ("SI_CAN_PURCHASE_AGAIN", "UStoreItemViewModel.bCanPurchaseAgain"),
    ("SI_EXPIRED", "UStoreItemViewModel.bIsExpired"),
    ("SI_PRESTIGE_ELITE", "UStoreItemViewModel.bIsPrestigeItemElite"),
    ("SI_RARITY", "UStoreItemViewModel.RewardRarityLevel"),
    ("SI_CATALOG_ITEM_ID", "UStoreItemViewModel.StoreItemCatalogData.ItemId"),
    // the offer, relative to the offer struct
    ("OF_SKU", "FOnlineStorePhoenixOffer.SkuId"),
    ("OF_DISPLAY_NAME", "FOnlineStorePhoenixOffer.DisplayName"),
    ("OF_DISPLAY_DESCRIPTION", "FOnlineStorePhoenixOffer.DisplayDescription"),
    ("OF_PRIORITY", "FOnlineStorePhoenixOffer.DisplayPriority"),
    ("OF_CATALOG_ID", "FOnlineStorePhoenixOffer.CatalogId"),
    ("OF_IMAGE_URL", "FOnlineStorePhoenixOffer.ImageURL"),
    ("OF_PLAT_PRICE", "FOnlineStorePhoenixOffer.PlatPrice"),
    ("OF_REGULAR_PLAT_PRICE", "FOnlineStorePhoenixOffer.RegularPlatPrice"),
    ("OF_DUST_PRICE", "FOnlineStorePhoenixOffer.DustPrice"),
    ("OF_MAX_ALLOWED", "FOnlineStorePhoenixOffer.MaxAllowed"),
    ("OF_REMAINING", "FOnlineStorePhoenixOffer.Remaining"),
    ("OF_TAGS", "FOnlineStorePhoenixOffer.Tags"),
    ("OF_GRANTED_IDS", "FOnlineStorePhoenixOffer.GrantedCatalogIds"),
    (
        "OF_GRANTED_ID_ITEM",
        "FOnlineStorePhoenixOffer.GrantedCatalogIds[].ItemId",
    ),
    (
        "OF_GRANTED_ID_AMOUNT",
        "FOnlineStorePhoenixOffer.GrantedCatalogIds[].Amount",
    ),
    (
        "OF_GRANTED_ENTITLEMENTS",
        "FOnlineStorePhoenixOffer.GrantedEntitlements",
    ),
    ("OF_GRANTED_TRACKS", "FOnlineStorePhoenixOffer.GrantedProgressInTracks"),
    (
        "OF_GRANTED_PROGRESS_TRACK",
        "FOnlineStorePhoenixOffer.GrantedProgress.TrackId",
    ),
    (
        "OF_GRANTED_PROGRESS_AMOUNT",
        "FOnlineStorePhoenixOffer.GrantedProgress.Amount",
    ),
    (
        "OF_GRANTED_PROGRESS_RANKS",
        "FOnlineStorePhoenixOffer.GrantedProgress.Ranks",
    ),
    ("OF_LOADOUT_SLOTS", "FOnlineStorePhoenixOffer.GrantedLoadoutSlots"),
    ("OF_ALL_CURRENCIES", "FOnlineStorePhoenixOffer.bPayWithAllCurrencies"),
    ("OF_CURRENCY_CODE", "FOnlineStorePhoenixOffer.CurrencyCode"),
    ("OF_CURRENCY_ENUM", "FOnlineStorePhoenixOffer.Currency"),
    ("OF_IN_COLLECTION", "FOnlineStorePhoenixOffer.bIsInUserCollection"),
    ("OF_PLATFORM_OFFER_ID", "FOnlineStorePhoenixOffer.PlatformOfferId"),
    ("OF_TIME_REASON", "FOnlineStorePhoenixOffer.TimeAvailabilityReason"),
    (
        "OF_MISSING_ENTITLEMENTS",
        "FOnlineStorePhoenixOffer.MissingEntitlementNames",
    ),
    ("OF_PREVIEW_IDS", "FOnlineStorePhoenixOffer.PreviewItemIds"),
    // store item images
    ("IMG_FEATURE", "FStoreItemTable.FeatureImage"),
    ("IMG_STANDARD", "FStoreItemTable.StandardImage"),
    // Progression (spec §6.6), beside the base groups below
    ("EXP_PRESTIGE_TRACK", "FExperienceTrackTableData.PrestigeTrack.RowName"),
    (
        "EXP_SHORT_NAME",
        "FExperienceTrackTableData.ExperienceTypeDisplayData.ShortDisplayName",
    ),
    (
        "EXP_NAME",
        "FExperienceTrackTableData.ExperienceTypeDisplayData.DisplayName",
    ),
    (
        "EXP_DESCRIPTION",
        "FExperienceTrackTableData.ExperienceTypeDisplayData.Description",
    ),
    (
        "EXP_BANKING_LEVELS",
        "FExperienceTrackTableData.ExperienceBankingDataPerLevel",
    ),
    ("HP_MUST_CLAIM", "FHuntPassSeasonDataTable.bMustClaimRewards"),
    ("HP_TITLE", "FHuntPassSeasonDataTable.SeasonTitle"),
    ("HP_DESCRIPTION", "FHuntPassSeasonDataTable.SeasonDescription"),
    ("HP_DATE", "FHuntPassSeasonDataTable.SeasonDate"),
    ("HP_REWARD_ITEM", "FHuntPassSeasonDataTable.HuntRewardItemId"),
    ("HP_REWARD_AMOUNT", "FHuntPassSeasonDataTable.HuntRewardItemAmount"),
    ("HP_NEXT_TRACK", "FHuntPassSeasonDataTable.NextProgressionTrack"),
    ("HP_PREVIOUS_TRACK", "FHuntPassSeasonDataTable.PreviewsProgressionTrack"),
    ("HP_BUY_WITH_RANK", "FHuntPassSeasonDataTable.bBuyLevelsUsingRank"),
    (
        "HP_SEASONAL_CURRENCIES",
        "FHuntPassSeasonDataTable.bGiveSeasonalCurrencies",
    ),
    ("HP_OVERALL_NAME", "FHuntPassSeasonDataTable.OverallSeasonName"),
    ("AW_TYPE", "FExperienceTableData.ExperienceType"),
    ("AW_AMOUNT", "FExperienceTableData.Amount"),
    ("AW_OBJECTIVES", "FExperienceTableData.Objectives"),
    ("PEC_TABLE", "UPlayerExperienceComponent.ExperienceTable"),
    ("PT_DISPLAY_NAME", "FProgressionTrackTableData.DisplayName"),
    ("PT_VERSION", "FProgressionTrackTableData.CurrentVersion"),
    ("PT_REPEATS", "FProgressionTrackTableData.Repeats"),
    ("PT_IS_PRESTIGE", "FProgressionTrackTableData.IsPrestigeTrack"),
    ("PT_IS_CORE", "FProgressionTrackTableData.IsCoreTrack"),
    ("PT_VISIBLE", "FProgressionTrackTableData.VisibleInUI"),
    ("PT_PRESTIGE_TRACK", "FProgressionTrackTableData.PrestigeTrack"),
    ("PT_RANK_TABLE", "FProgressionTrackTableData.RankTable"),
    ("PT_UNLOCK_CONDITIONS", "FProgressionTrackTableData.UnlockConditions"),
    ("PT_CHALLENGE_SLOTS", "FProgressionTrackTableData.ChallengeSlots"),
    ("PT_CRAFT_REWARD", "FProgressionTrackTableData.CraftItemRewardAmount"),
    (
        "PT_UPGRADE_REWARD",
        "FProgressionTrackTableData.UpgradeItemRewardAmount",
    ),
    ("OT_DISPLAY_NAME", "FOnlineProgressionTrackInfo.DisplayName"),
    ("OT_SINGULAR", "FOnlineProgressionTrackInfo.ProgressDisplaySingular"),
    ("OT_PLURAL", "FOnlineProgressionTrackInfo.ProgressDisplayPlural"),
    ("OT_DESCRIPTION", "FOnlineProgressionTrackInfo.ProgressDescription"),
    ("MT_DISPLAY_NAME", "FMasteryTrackTableData.DisplayName"),
    ("MT_SHORT_NAME", "FMasteryTrackTableData.ShortDisplayName"),
    ("MT_UI_ENABLED", "FMasteryTrackTableData.bIsUIEnabled"),
    ("MT_CATEGORY", "FMasteryTrackTableData.Categtory"),
    // Mastery (spec §6.14)
    ("MC_OBJECTIVES", "FMasteryCardTableData.Objectives"),
    (
        "MC_OBJ_ID",
        "FMasteryCardTableData.Objectives[].UniqueOnlineObjectiveId",
    ),
    ("MC_OBJ_TARGET", "FMasteryCardTableData.Objectives[].Objective.Amount"),
    (
        "MC_OBJ_DATA",
        "FMasteryCardTableData.Objectives[].Objective.ObjectiveDataClass",
    ),
    (
        "MC_OBJ_COMPOUND",
        "FMasteryCardTableData.Objectives[].CompoundObjectiveId",
    ),
    (
        "MC_OBJ_RANK",
        "FMasteryCardTableData.Objectives[].Objective.ProgressRank",
    ),
    (
        "MC_OBJ_TRACK",
        "FMasteryCardTableData.Objectives[].Objective.ProgressTrack",
    ),
    ("MC_REWARD_TRACKS", "FMasteryCardTableData.RewardTracks"),
    ("MC_CARD_TYPE", "FMasteryCardTableData.CardType"),
    ("MC_GAME_OBJECT", "FMasteryCardTableData.GameObjectId"),
    ("MC_BASE_XP", "FMasteryCardTableData.BaseXPBarOnlineObjectiveId"),
    ("MC_PRESTIGE_XP", "FMasteryCardTableData.PrestigeXPBarOnlineObjectiveId"),
    ("AC_OBJECTIVES", "FAchievementTableData.TrackedObjectives"),
    (
        "AC_OBJ_ID",
        "FAchievementTableData.TrackedObjectives[].UniqueOnlineObjectiveId",
    ),
    (
        "AC_OBJ_TARGET",
        "FAchievementTableData.TrackedObjectives[].Objective.Amount",
    ),
    (
        "AC_OBJ_DATA",
        "FAchievementTableData.TrackedObjectives[].Objective.ObjectiveDataClass",
    ),
    (
        "AC_OBJ_RANK",
        "FAchievementTableData.TrackedObjectives[].Objective.ProgressRank",
    ),
    (
        "AC_OBJ_TRACK",
        "FAchievementTableData.TrackedObjectives[].Objective.ProgressTrack",
    ),
];

const OBJECT_PTR: Kind = Kind::ObjPtr;
const INT32: Kind = Kind::Int { size: 4, signed: true };

/// `(id, path, default)`: members an SDK may not carry, with UE 4.26's own layout, or the spec's fixed offset
/// (§4.5), when it does not.
pub const OPTIONAL_MEMBERS: &[(&str, &str, Member)] = &[
    (
        "OBJECT_ARRAY_OBJECTS",
        "TUObjectArray.Objects",
        Member::plain(0, 8, OBJECT_PTR),
    ),
    (
        "OBJECT_ARRAY_NUM",
        "TUObjectArray.NumElements",
        Member::plain(0x14, 4, INT32),
    ),
    (
        "OBJECT_ITEM_OBJECT",
        "FUObjectItem.Object",
        Member::plain(0, 8, OBJECT_PTR),
    ),
    (
        "OBJECT_ITEM_SERIAL",
        "FUObjectItem.SerialNumber",
        Member::plain(0x10, 4, INT32),
    ),
    (
        "SV_TAG_FILTER",
        "UStoreViewModel.TagFilter",
        Member::plain(0x170, 16, Kind::Str),
    ),
    (
        "SI_OFFER",
        "UStoreItemViewModel.StoreItem",
        Member::plain(0x650, 0, Kind::Struct),
    ),
];

/// `(id, struct)`: sizes from the struct's header comment.
pub const SIZES: &[(&str, &str)] = &[("GET_ALL_ITEMS_PARAMS", "ArchonCatalog_GetAllItems")];

/// `(id, struct, default)`.
pub const OPTIONAL_SIZES: &[(&str, &str, u32)] = &[("OBJECT_ITEM_SIZE", "FUObjectItem", 0x18)];

/// `(id, name, default)`: `Basic.hpp` constants; `None` is required.
pub const CONSTS: &[(&str, &str, Option<i64>)] = &[
    ("GOBJECTS", "Offsets::GObjects", None),
    ("GNAMES", "Offsets::GNames", None),
    ("PROCESS_EVENT_INDEX", "Offsets::ProcessEventIdx", None),
    ("ELEMENTS_PER_CHUNK", "ElementsPerChunk", Some(0x10000)),
];

/// `(id, path)`: enum values, by enumerator name (spec §6.14).
pub const ENUMS: &[(&str, &str)] = &[
    ("CARD_TYPE_WEAPON", "FMasteryCardTableData.CardType::Weapon"),
    ("CARD_TYPE_BEHEMOTH", "FMasteryCardTableData.CardType::Behemoth"),
];

/// Where each row struct's shared progression base is (spec §6.6): inherited members today; when the real SDK nests
/// them, add the member to the path.
pub const BASE_OWNERS: &[(&str, &str)] = &[
    ("EXP_BASE", "FExperienceTrackTableData"),
    ("HP_BASE", "FHuntPassSeasonDataTable"),
    ("MT_BASE", "FMasteryTrackTableData"),
];

/// The base's members, in this order; `B_*` index them.
pub const BASE_MEMBERS: &[&str] = &[
    "ProgressionTrack",
    "PremiumGatingEntitlement",
    "ResetWithCharacter",
    "ProgressionMultiplier",
    "Requirements",
    "Requirements[].RankId",
    "Requirements[].RequiredXP",
    "FreeRewards",
    "PremiumRewards",
    "FreeRewards[].RankId",
    "FreeRewards[].CustomRewardString",
    "FreeRewards[].StackedItems",
    "FreeRewards[].StackedItems[].CatalogId",
    "FreeRewards[].StackedItems[].Quantity",
    "FreeRewards[].StackedItems[].Priority",
    "FreeRewards[].InstancedItems",
    "FreeRewards[].OrderedInstancedItems",
    "FreeRewards[].OrderedInstancedItems[].CatalogId",
    "FreeRewards[].OrderedInstancedItems[].Priority",
    "FreeRewards[].Entitlements",
    "FreeRewards[].Entitlements[].EntitlementId",
    "FreeRewards[].Entitlements[].Duration",
    "FreeRewards[].Buffs",
];

pub const B_TRACK: usize = 0;
pub const B_GATING: usize = 1;
pub const B_RESET: usize = 2;
pub const B_MULTIPLIER: usize = 3;
pub const B_REQUIREMENTS: usize = 4;
pub const B_REQ_RANK: usize = 5;
pub const B_REQ_XP: usize = 6;
pub const B_FREE: usize = 7;
pub const B_PREMIUM: usize = 8;
pub const B_RW_RANK: usize = 9;
pub const B_RW_CUSTOM: usize = 10;
pub const B_RW_STACKED: usize = 11;
pub const B_ST_ID: usize = 12;
pub const B_ST_QUANTITY: usize = 13;
pub const B_ST_PRIORITY: usize = 14;
pub const B_RW_INSTANCED: usize = 15;
pub const B_RW_ORDERED: usize = 16;
pub const B_OR_ID: usize = 17;
pub const B_OR_PRIORITY: usize = 18;
pub const B_RW_ENTITLEMENTS: usize = 19;
pub const B_EN_ID: usize = 20;
pub const B_EN_DURATION: usize = 21;
pub const B_RW_BUFFS: usize = 22;
pub const BASE_LEN: usize = 23;
const _: () = assert!(BASE_MEMBERS.len() == BASE_LEN);
