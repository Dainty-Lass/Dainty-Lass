// Synthetic SDK for the catalog exporter's tests: made up, never generated from a game.

#pragma once

// Enum Synthetic.ECurrencyType
// NumValues: 0x0003
enum class ECurrencyType : uint8
{
	ECurrencyType_None                                     = 0,
	ECurrencyType_Notes                                    = 1,
	ECurrencyType_Platinum                                 = 2,
};

// Enum Synthetic.EExperienceType
// NumValues: 0x0003
enum class EExperienceType : uint8
{
	EExperienceType_None                                     = 0,
	EExperienceType_Player                                   = 1,
	EExperienceType_Weapon                                   = 2,
};

// Enum Synthetic.EMasteryCardType
// NumValues: 0x0004
enum class EMasteryCardType : uint8
{
	EMasteryCardType_None                                     = 0,
	EMasteryCardType_Weapon                                   = 1,
	EMasteryCardType_Behemoth                                 = 2,
	EMasteryCardType_MAX                                      = 3,
};

// ScriptStruct Synthetic.ArchonCatalogItemQuantity
// 0x0018 (0x0018 - 0x0000)
struct FArchonCatalogItemQuantity
{
public:
	class FString                                 Item;                                              // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         Amount;                                            // 0x0010(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.VirtualCurrencyPrice
// 0x0008 (0x0008 - 0x0000)
struct FVirtualCurrencyPrice
{
public:
	ECurrencyType                                 Currency;                                          // 0x0000(0x0001)(Edit, NativeAccessSpecifierPublic)
	int32                                         Amount;                                            // 0x0004(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.ArchonCatalogItem
// 0x0168 (0x0168 - 0x0000)
struct FArchonCatalogItem
{
public:
	class FString                                 ItemId;                                            // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	TSoftClassPtr<class UObject>                  ItemClass;                                         // 0x0010(0x0028)(Edit, NativeAccessSpecifierPublic)
	class FString                                 DisplayNameInvariant;                              // 0x0038(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FText                                   DisplayName;                                       // 0x0048(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FString                                 DescriptionInvariant;                              // 0x0060(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         Tags;                                              // 0x0070(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 CustomData;                                        // 0x0080(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         Entitlements;                                      // 0x0090(0x0010)(Edit, NativeAccessSpecifierPublic)
	uint8                                         bIsStackable : 1;                                  // 0x00A0(0x0001)(BitIndex: 0x00, PropSize: 0x0001 (Edit, NativeAccessSpecifierPublic))
	uint8                                         bIsBundle : 1;                                     // 0x00A0(0x0001)(BitIndex: 0x01, PropSize: 0x0001 (Edit, NativeAccessSpecifierPublic))
	uint8                                         bIsContainer : 1;                                  // 0x00A0(0x0001)(BitIndex: 0x02, PropSize: 0x0001 (Edit, NativeAccessSpecifierPublic))
	int32                                         MaxQuantity;                                       // 0x00A4(0x0004)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         ContainerItemContents;                             // 0x00A8(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         BundledItems;                                      // 0x00B8(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FArchonCatalogItemQuantity>     ContainerItemContentsWithQuantity;                 // 0x00C8(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         ContainerResultTableContents;                      // 0x00D8(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FArchonCatalogItemQuantity>     ContainerResultTableContentsWithQuantity;          // 0x00E8(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FArchonCatalogItemQuantity>     BundledItemsWithQuantity;                          // 0x00F8(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         BundledResultTables;                               // 0x0108(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FArchonCatalogItemQuantity>     BundledResultTablesWithQuantity;                   // 0x0118(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FArchonCatalogItemQuantity>     OtherItemQuantities;                               // 0x0128(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FArchonCatalogItemQuantity>     AlternateItemsToGrant;                             // 0x0138(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FVirtualCurrencyPrice>          VirtualCurrencyPrices;                             // 0x0148(0x0010)(Edit, NativeAccessSpecifierPublic)
	struct FDataTableRowHandle                    PlayerJourneyUnlockRow;                            // 0x0158(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.JourneyCurrencyCost
// 0x0018 (0x0018 - 0x0000)
struct FJourneyCurrencyCost
{
public:
	struct FDataTableRowHandle                    Currency;                                          // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         Amount;                                            // 0x0010(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.JourneyExperienceCost
// 0x0008 (0x0008 - 0x0000)
struct FJourneyExperienceCost
{
public:
	EExperienceType                               ExperienceType;                                    // 0x0000(0x0001)(Edit, NativeAccessSpecifierPublic)
	int32                                         Amount;                                            // 0x0004(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.JourneyReward
// 0x0030 (0x0030 - 0x0000)
struct FJourneyReward
{
public:
	class FName                                   ItemId;                                            // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	int32                                         Amount;                                            // 0x0008(0x0004)(Edit, NativeAccessSpecifierPublic)
	class FName                                   EntitlementId;                                     // 0x000C(0x0008)(Edit, NativeAccessSpecifierPublic)
	int32                                         Duration;                                          // 0x0014(0x0004)(Edit, NativeAccessSpecifierPublic)
	class FString                                 PassPrefix;                                        // 0x0018(0x0010)(Edit, NativeAccessSpecifierPublic)
	EExperienceType                               ExperienceType;                                    // 0x0028(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bAutoEquip;                                        // 0x0029(0x0001)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.PlayerJourneyNodeData
// 0x00E8 (0x00E8 - 0x0000)
struct FPlayerJourneyNodeData
{
public:
	uint8                                         NodeType;                                          // 0x0000(0x0001)(Edit, NativeAccessSpecifierPublic)
	int32                                         NodeLevel;                                         // 0x0004(0x0004)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FDataTableRowHandle>            ChildNodes;                                        // 0x0008(0x0010)(Edit, NativeAccessSpecifierPublic)
	uint8                                         bAutoUnlockIfParentUnlocked : 1;                   // 0x0018(0x0001)(BitIndex: 0x00, PropSize: 0x0001 (Edit, NativeAccessSpecifierPublic))
	TArray<struct FJourneyCurrencyCost>           CurrencyCosts;                                     // 0x0020(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FJourneyExperienceCost>         ExperienceCosts;                                   // 0x0030(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FJourneyReward>                 Rewards;                                           // 0x0040(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class UObject*>                        SystemRewards;                                     // 0x0050(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FName                                   ActivationEventId;                                 // 0x0060(0x0008)(Edit, NativeAccessSpecifierPublic)
	TArray<class FName>                           QuestIds;                                          // 0x0068(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FName                                   QuestIdToNotifyOnUnlock;                           // 0x0078(0x0008)(Edit, NativeAccessSpecifierPublic)
	class FName                                   TutorialToShowWhenUnlocked;                        // 0x0080(0x0008)(Edit, NativeAccessSpecifierPublic)
	TArray<class UObject*>                        Objectives;                                        // 0x0088(0x0010)(Edit, NativeAccessSpecifierPublic)
	TMap<struct FGameplayAttribute, float>        GameplayAttributeValues;                           // 0x0098(0x0050)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.MatchmakerHuntList
// 0x0018 (0x0018 - 0x0000)
struct FMatchmakerHuntList
{
public:
	class UDataTable*                             MatchmakerTable;                                   // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FGameplayTagQuery>              Queries;                                           // 0x0008(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.PlayerHuntTableData
// 0x00E8 (0x00E8 - 0x0000)
struct FPlayerHuntTableData
{
public:
	class FString                                 PlayerHuntId;                                      // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	struct FDataTableRowHandle                    Region;                                            // 0x0010(0x0010)(Edit, NativeAccessSpecifierPublic)
	uint8                                         MatchmakingType;                                   // 0x0020(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         MatchmakingGameType;                               // 0x0021(0x0001)(Edit, NativeAccessSpecifierPublic)
	struct FGameplayTagContainer                  HuntTags;                                          // 0x0028(0x0020)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FDataTableRowHandle>            MatchmakerHuntIDs;                                 // 0x0048(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FMatchmakerHuntList>            MatchmakerHuntsByTag;                              // 0x0058(0x0010)(Edit, NativeAccessSpecifierPublic)
	struct FDataTableRowHandle                    EscalationModeSpecification;                       // 0x0068(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         EscalationPatrolInitialChallengeLevel;             // 0x0078(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         EscalationPatrolMaxChallengeLevel;                 // 0x007C(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         RecommendedEscalationLevel;                        // 0x0080(0x0004)(Edit, NativeAccessSpecifierPublic)
	bool                                          bHasPortals;                                       // 0x0084(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bHasGlitterEvent;                                  // 0x0085(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bHasPhaelanxEvent;                                 // 0x0086(0x0001)(Edit, NativeAccessSpecifierPublic)
	class FString                                 HuntSuccessReward;                                 // 0x0088(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 FirstCompletionSuccessReward;                      // 0x0098(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 TargetedHuntSuccessReward;                         // 0x00A8(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 HuntFailureReward;                                 // 0x00B8(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 TargetedHuntFailureReward;                         // 0x00C8(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         MinWeaponSkillLevel;                               // 0x00D8(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         RecomendedWeaponSkillLevel;                        // 0x00DC(0x0004)(Edit, NativeAccessSpecifierPublic)
	bool                                          bVisibleWhileLocked;                               // 0x00E0(0x0001)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.BehemothSpawnInfo
// 0x0038 (0x0038 - 0x0000)
struct FBehemothSpawnInfo
{
public:
	class FName                                   BehemothName;                                      // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	TSoftObjectPtr<class UObject>                 BehemothAsset;                                     // 0x0008(0x0028)(Edit, NativeAccessSpecifierPublic)
	int32                                         PowerOverride;                                     // 0x0030(0x0004)(Edit, NativeAccessSpecifierPublic)
	float                                         Weighting;                                         // 0x0034(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.HuntMapInfo
// 0x0040 (0x0040 - 0x0000)
struct FHuntMapInfo
{
public:
	class FName                                   MapName;                                           // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	class FName                                   MapAssetName;                                      // 0x0008(0x0008)(Edit, NativeAccessSpecifierPublic)
	TSoftObjectPtr<class UObject>                 MapAsset;                                          // 0x0010(0x0028)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Biome;                                             // 0x0038(0x0001)(Edit, NativeAccessSpecifierPublic)
	float                                         Weighting;                                         // 0x003C(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.MatchmakerHuntTableData
// 0x00F0 (0x00F0 - 0x0000)
struct FMatchmakerHuntTableData
{
public:
	struct FDataTableRowHandle                    Region;                                            // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	struct FGameplayTagContainer                  HuntTags;                                          // 0x0010(0x0020)(Edit, NativeAccessSpecifierPublic)
	int32                                         HuntThreatLevel;                                   // 0x0030(0x0004)(Edit, NativeAccessSpecifierPublic)
	class FString                                 GameModeOverride;                                  // 0x0038(0x0010)(Edit, NativeAccessSpecifierPublic)
	float                                         DangerPerSecOverride;                              // 0x0048(0x0004)(Edit, NativeAccessSpecifierPublic)
	struct FBehemothSpawnInfo                     SpecificBehemoth;                                  // 0x0050(0x0038)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FBehemothSpawnInfo>             AdditionalSpecificBehemoths;                       // 0x0088(0x0010)(Edit, NativeAccessSpecifierPublic)
	struct FDataTableRowHandle                    MapMetaData;                                       // 0x0098(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FHuntMapInfo>                   MapList;                                           // 0x00A8(0x0010)(Edit, NativeAccessSpecifierPublic)
	class UObject*                                SpecificModifier;                                  // 0x00B8(0x0008)(Edit, NativeAccessSpecifierPublic)
	TArray<class FName>                           Modifiers;                                         // 0x00C0(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         GatherableTier;                                    // 0x00D0(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         MaxPlayers;                                        // 0x00D4(0x0004)(Edit, NativeAccessSpecifierPublic)
	bool                                          bIsGeneratedEncounter;                             // 0x00D8(0x0001)(Edit, NativeAccessSpecifierPublic)
	struct FDataTableRowHandle                    GameModeSpecificData;                              // 0x00E0(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.Hunt_Region
// 0x0038 (0x0038 - 0x0000)
struct FHunt_Region
{
public:
	class FName                                   MatchmakingId;                                     // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	class FString                                 TokenId;                                           // 0x0008(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         NumTokensRequired;                                 // 0x0018(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         NumHuntPassBehemothRewards;                        // 0x001C(0x0004)(Edit, NativeAccessSpecifierPublic)
	class UObject*                                TargetedBehemothList;                              // 0x0020(0x0008)(Edit, NativeAccessSpecifierPublic)
	TArray<TSoftObjectPtr<class UObject>>         BehemothPresets;                                   // 0x0028(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.EscalationRound
// 0x0004 (0x0004 - 0x0000)
struct FEscalationRound
{
public:
	int32                                         Encounters;                                        // 0x0000(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.EscalationModeSpecification
// 0x0078 (0x0078 - 0x0000)
struct FEscalationModeSpecification
{
public:
	int32                                         InitialChallengeLevel;                             // 0x0000(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         MaxChallengeLevel;                                 // 0x0004(0x0004)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FEscalationRound>               RoundStructure;                                    // 0x0008(0x0010)(Edit, NativeAccessSpecifierPublic)
	struct FDataTableRowHandle                    LootTable;                                         // 0x0018(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         OptionalPowerOverride;                             // 0x0028(0x0004)(Edit, NativeAccessSpecifierPublic)
	struct FGameplayTagContainer                  EscalationModeTags;                                // 0x0030(0x0020)(Edit, NativeAccessSpecifierPublic)
	class UDataTable*                             BackupEncountersTable;                             // 0x0050(0x0008)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         CompletionRewards;                                 // 0x0058(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<float>                                 EscalationAmountPerScoreValue;                     // 0x0068(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.HuntModifierTableRow
// 0x0060 (0x0060 - 0x0000)
struct FHuntModifierTableRow
{
public:
	TSoftObjectPtr<class UObject>                 ModifierAsset;                                     // 0x0000(0x0028)(Edit, NativeAccessSpecifierPublic)
	int32                                         HuntComplexityModifier;                            // 0x0028(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         ThreatLevel;                                       // 0x002C(0x0004)(Edit, NativeAccessSpecifierPublic)
	uint8                                         ModifierApplicationType;                           // 0x0030(0x0001)(Edit, NativeAccessSpecifierPublic)
	struct FGameplayTagContainer                  ModifierMetaTags;                                  // 0x0038(0x0020)(Edit, NativeAccessSpecifierPublic)
	uint8                                         BehemothElementCondition;                          // 0x0058(0x0001)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.GameActivityTableData
// 0x0038 (0x0038 - 0x0000)
struct FGameActivityTableData
{
public:
	bool                                          bEnabled;                                          // 0x0000(0x0001)(Edit, NativeAccessSpecifierPublic)
	class FName                                   PlayerHunt;                                        // 0x0004(0x0008)(Edit, NativeAccessSpecifierPublic)
	TArray<class FName>                           MatchmakerHunts;                                   // 0x0010(0x0010)(Edit, NativeAccessSpecifierPublic)
	uint8                                         UnlockRequirement;                                 // 0x0020(0x0001)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FDataTableRowHandle>            UnlockCriteria;                                    // 0x0028(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.TrackRank
// 0x000C (0x000C - 0x0000)
struct FTrackRank
{
public:
	class FName                                   Track;                                             // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	int32                                         Rank;                                              // 0x0008(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.QuestRequirement
// 0x000C (0x000C - 0x0000)
struct FQuestRequirement
{
public:
	class FName                                   QuestName;                                         // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	uint8                                         StatusRequired;                                    // 0x0008(0x0001)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.ProgressionUnlockCriteria
// 0x0078 (0x0078 - 0x0000)
struct FProgressionUnlockCriteria
{
public:
	uint8                                         FeatureFlagsOperation;                             // 0x0000(0x0001)(Edit, NativeAccessSpecifierPublic)
	TArray<TSoftObjectPtr<class UObject>>         FeatureFlags;                                      // 0x0008(0x0010)(Edit, NativeAccessSpecifierPublic)
	uint8                                         ItemIdsOperation;                                  // 0x0018(0x0001)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         ItemIds;                                           // 0x0020(0x0010)(Edit, NativeAccessSpecifierPublic)
	uint8                                         ProgressionTrackRanksOperation;                    // 0x0030(0x0001)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FTrackRank>                     ProgressionTrackRanks;                             // 0x0038(0x0010)(Edit, NativeAccessSpecifierPublic)
	uint8                                         QuestRequirementsOperation;                        // 0x0048(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         QuestRequirementsCondition;                        // 0x0049(0x0001)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FQuestRequirement>              QuestRequirements;                                 // 0x0050(0x0010)(Edit, NativeAccessSpecifierPublic)
	uint8                                         PlayerJourneyNodesOperation;                       // 0x0060(0x0001)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FDataTableRowHandle>            PlayerJourneyNodes;                                // 0x0068(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.ScheduledItem
// 0x000C (0x000C - 0x0000)
struct FScheduledItem
{
public:
	class FName                                   Id;                                                // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	int32                                         MaxCompletionPerInterval;                          // 0x0008(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.ScheduleData
// 0x0038 (0x0038 - 0x0000)
struct FScheduleData
{
public:
	struct FDateTime                              StartTime;                                         // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	struct FDateTime                              EndTime;                                           // 0x0008(0x0008)(Edit, NativeAccessSpecifierPublic)
	uint8                                         bIsRepeatable : 1;                                 // 0x0010(0x0001)(BitIndex: 0x00, PropSize: 0x0001 (Edit, NativeAccessSpecifierPublic))
	uint8                                         bEachDay : 1;                                      // 0x0010(0x0001)(BitIndex: 0x01, PropSize: 0x0001 (Edit, NativeAccessSpecifierPublic))
	uint8                                         bEachWeek : 1;                                     // 0x0010(0x0001)(BitIndex: 0x02, PropSize: 0x0001 (Edit, NativeAccessSpecifierPublic))
	uint8                                         bShouldQueue : 1;                                  // 0x0010(0x0001)(BitIndex: 0x03, PropSize: 0x0001 (Edit, NativeAccessSpecifierPublic))
	TArray<struct FScheduledItem>                 ScheduledItems;                                    // 0x0018(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<int32>                                 ItemsLeftOut;                                      // 0x0028(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.StoreSubcategory
// 0x0060 (0x0060 - 0x0000)
struct FStoreSubcategory
{
public:
	class FText                                   Name;                                              // 0x0000(0x0018)(Edit, NativeAccessSpecifierPublic)
	struct FGameplayTag                           BreadcrumbCategory;                                // 0x0018(0x0008)(Edit, NativeAccessSpecifierPublic)
	bool                                          bRequiresEntitlement;                              // 0x0020(0x0001)(Edit, NativeAccessSpecifierPublic)
	class FString                                 EntitlementId;                                     // 0x0028(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 SkuId;                                             // 0x0038(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         SubTagIds;                                         // 0x0048(0x0010)(Edit, NativeAccessSpecifierPublic)
	bool                                          bHidePurchasedItems;                               // 0x0058(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bShouldHideTab;                                    // 0x0059(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bHideTab;                                          // 0x005A(0x0001)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.StoreCategory
// 0x0058 (0x0058 - 0x0000)
struct FStoreCategory
{
public:
	class FText                                   Name;                                              // 0x0000(0x0018)(Edit, NativeAccessSpecifierPublic)
	struct FGameplayTag                           BreadcrumbCategory;                                // 0x0018(0x0008)(Edit, NativeAccessSpecifierPublic)
	bool                                          bRequiresEntitlement;                              // 0x0020(0x0001)(Edit, NativeAccessSpecifierPublic)
	class FString                                 EntitlementId;                                     // 0x0028(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 SkuId;                                             // 0x0038(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FStoreSubcategory>              Subcategories;                                     // 0x0048(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.GrantedItem
// 0x0018 (0x0018 - 0x0000)
struct FGrantedItem
{
public:
	class FString                                 ItemId;                                            // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         Amount;                                            // 0x0010(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.GrantedProgress
// 0x0018 (0x0018 - 0x0000)
struct FGrantedProgress
{
public:
	class FString                                 TrackId;                                           // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         Amount;                                            // 0x0010(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         Ranks;                                             // 0x0014(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.OnlineStorePhoenixOffer
// 0x0128 (0x0128 - 0x0000)
struct FOnlineStorePhoenixOffer
{
public:
	class FString                                 SkuId;                                             // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 DisplayName;                                       // 0x0010(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 DisplayDescription;                                // 0x0020(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         DisplayPriority;                                   // 0x0030(0x0004)(Edit, NativeAccessSpecifierPublic)
	class FString                                 CatalogId;                                         // 0x0038(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 ImageUrl;                                          // 0x0048(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         PlatPrice;                                         // 0x0058(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         RegularPlatPrice;                                  // 0x005C(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         DustPrice;                                         // 0x0060(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         MaxAllowed;                                        // 0x0064(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         Remaining;                                         // 0x0068(0x0004)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         Tags;                                              // 0x0070(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FGrantedItem>                   GrantedCatalogIds;                                 // 0x0080(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         GrantedEntitlements;                               // 0x0090(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         GrantedProgressInTracks;                           // 0x00A0(0x0010)(Edit, NativeAccessSpecifierPublic)
	struct FGrantedProgress                       GrantedProgress;                                   // 0x00B0(0x0018)(Edit, NativeAccessSpecifierPublic)
	int32                                         GrantedLoadoutSlots;                               // 0x00C8(0x0004)(Edit, NativeAccessSpecifierPublic)
	bool                                          bPayWithAllCurrencies;                             // 0x00CC(0x0001)(Edit, NativeAccessSpecifierPublic)
	class FString                                 CurrencyCode;                                      // 0x00D0(0x0010)(Edit, NativeAccessSpecifierPublic)
	uint8                                         CurrencyEnum;                                      // 0x00E0(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bInUserCollection;                                 // 0x00E1(0x0001)(Edit, NativeAccessSpecifierPublic)
	class FString                                 PlatformOfferId;                                   // 0x00E8(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 TimeAvailabilityReason;                            // 0x00F8(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         MissingEntitlementNames;                           // 0x0108(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         PreviewItemIds;                                    // 0x0118(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.StoreItemTable
// 0x0050 (0x0050 - 0x0000)
struct FStoreItemTable
{
public:
	TSoftObjectPtr<class UObject>                 FeatureImage;                                      // 0x0000(0x0028)(Edit, NativeAccessSpecifierPublic)
	TSoftObjectPtr<class UObject>                 StandardImage;                                     // 0x0028(0x0028)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.RankRequirement
// 0x0008 (0x0008 - 0x0000)
struct FRankRequirement
{
public:
	int32                                         RankId;                                            // 0x0000(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         RequiredXP;                                        // 0x0004(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.StackedItem
// 0x0018 (0x0018 - 0x0000)
struct FStackedItem
{
public:
	class FString                                 CatalogId;                                         // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         Quantity;                                          // 0x0010(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         Priority;                                          // 0x0014(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.OrderedItem
// 0x0018 (0x0018 - 0x0000)
struct FOrderedItem
{
public:
	class FString                                 CatalogId;                                         // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         Priority;                                          // 0x0010(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.EntitlementReward
// 0x0018 (0x0018 - 0x0000)
struct FEntitlementReward
{
public:
	class FString                                 EntitlementId;                                     // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         Duration;                                          // 0x0010(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.RankReward
// 0x0070 (0x0070 - 0x0000)
struct FRankReward
{
public:
	int32                                         RankId;                                            // 0x0000(0x0004)(Edit, NativeAccessSpecifierPublic)
	class FText                                   CustomRewardString;                                // 0x0008(0x0018)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FStackedItem>                   StackedItems;                                      // 0x0020(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class FString>                         InstancedItems;                                    // 0x0030(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FOrderedItem>                   OrderedInstancedItems;                             // 0x0040(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FEntitlementReward>             Entitlements;                                      // 0x0050(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class UObject*>                        Buffs;                                             // 0x0060(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.ProgressionTrackBase
// 0x0050 (0x0050 - 0x0000)
struct FProgressionTrackBase
{
public:
	class FName                                   ProgressionTrack;                                  // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	class FString                                 PremiumGatingEntitlement;                          // 0x0008(0x0010)(Edit, NativeAccessSpecifierPublic)
	bool                                          bResetWithCharacter;                               // 0x0018(0x0001)(Edit, NativeAccessSpecifierPublic)
	float                                         ProgressionMultiplier;                             // 0x001C(0x0004)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FRankRequirement>               Requirements;                                      // 0x0020(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FRankReward>                    FreeRewards;                                       // 0x0030(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FRankReward>                    PremiumRewards;                                    // 0x0040(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.ExperienceTypeDisplay
// 0x0048 (0x0048 - 0x0000)
struct FExperienceTypeDisplay
{
public:
	class FText                                   ShortDisplayName;                                  // 0x0000(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FText                                   DisplayName;                                       // 0x0018(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FText                                   DisplayDescription;                                // 0x0030(0x0018)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.ExperienceTrackTableData
// 0x0060 (0x00B0 - 0x0050)
struct FExperienceTrackTableData : public FProgressionTrackBase
{
public:
	struct FDataTableRowHandle                    PrestigeTrack;                                     // 0x0050(0x0010)(Edit, NativeAccessSpecifierPublic)
	struct FExperienceTypeDisplay                 ExperienceTypeDisplayData;                         // 0x0060(0x0048)(Edit, NativeAccessSpecifierPublic)
	int32                                         ExperienceBankingLevelCount;                       // 0x00A8(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.HuntPassSeasonDataTable
// 0x0090 (0x00E0 - 0x0050)
struct FHuntPassSeasonDataTable : public FProgressionTrackBase
{
public:
	bool                                          bMustClaimRewards;                                 // 0x0050(0x0001)(Edit, NativeAccessSpecifierPublic)
	class FText                                   SeasonTitle;                                       // 0x0058(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FText                                   SeasonDescription;                                 // 0x0070(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FText                                   SeasonDate;                                        // 0x0088(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FString                                 HuntRewardItemId;                                  // 0x00A0(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         HuntRewardItemAmount;                              // 0x00B0(0x0004)(Edit, NativeAccessSpecifierPublic)
	class FName                                   NextProgressionTrack;                              // 0x00B4(0x0008)(Edit, NativeAccessSpecifierPublic)
	class FName                                   PreviewsProgressionTrack;                          // 0x00BC(0x0008)(Edit, NativeAccessSpecifierPublic)
	bool                                          bBuyLevelsUsingRank;                               // 0x00C4(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bGiveSeasonalCurrencies;                           // 0x00C5(0x0001)(Edit, NativeAccessSpecifierPublic)
	class FText                                   OverallSeasonName;                                 // 0x00C8(0x0018)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.ExperienceTableData
// 0x0018 (0x0018 - 0x0000)
struct FExperienceTableData
{
public:
	EExperienceType                               ExperienceType;                                    // 0x0000(0x0001)(Edit, NativeAccessSpecifierPublic)
	int32                                         Amount;                                            // 0x0004(0x0004)(Edit, NativeAccessSpecifierPublic)
	TArray<class UObject*>                        Objectives;                                        // 0x0008(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.ProgressionTrackTableData
// 0x0058 (0x0058 - 0x0000)
struct FProgressionTrackTableData
{
public:
	class FText                                   DisplayName;                                       // 0x0000(0x0018)(Edit, NativeAccessSpecifierPublic)
	int32                                         CurrentVersion;                                    // 0x0018(0x0004)(Edit, NativeAccessSpecifierPublic)
	bool                                          bRepeats;                                          // 0x001C(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bIsPrestigeTrack;                                  // 0x001D(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bIsCoreTrack;                                      // 0x001E(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bVisibleInUI;                                      // 0x001F(0x0001)(Edit, NativeAccessSpecifierPublic)
	class FName                                   PrestigeTrack;                                     // 0x0020(0x0008)(Edit, NativeAccessSpecifierPublic)
	class UDataTable*                             RankTable;                                         // 0x0028(0x0008)(Edit, NativeAccessSpecifierPublic)
	TArray<class UObject*>                        UnlockConditions;                                  // 0x0030(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class UObject*>                        ChallengeSlots;                                    // 0x0040(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         CraftItemRewardAmount;                             // 0x0050(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         UpgradeItemRewardAmount;                           // 0x0054(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.OnlineProgressionTrackInfo
// 0x0060 (0x0060 - 0x0000)
struct FOnlineProgressionTrackInfo
{
public:
	class FText                                   DisplayName;                                       // 0x0000(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FText                                   ProgressDisplaySingular;                           // 0x0018(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FText                                   ProgressDisplayPlural;                             // 0x0030(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FText                                   ProgressDescription;                               // 0x0048(0x0018)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.MasteryTrackTableData
// 0x0038 (0x0088 - 0x0050)
struct FMasteryTrackTableData : public FProgressionTrackBase
{
public:
	class FText                                   DisplayName;                                       // 0x0050(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FText                                   ShortDisplayName;                                  // 0x0068(0x0018)(Edit, NativeAccessSpecifierPublic)
	bool                                          bIsUIEnabled;                                      // 0x0080(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Category;                                          // 0x0081(0x0001)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.MasteryObjective
// 0x0040 (0x0040 - 0x0000)
struct FMasteryObjective
{
public:
	class FString                                 UniqueOnlineObjectiveId;                           // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         Target;                                            // 0x0010(0x0004)(Edit, NativeAccessSpecifierPublic)
	class UClass*                                 ObjectiveData;                                     // 0x0018(0x0008)(Edit, NativeAccessSpecifierPublic)
	class FString                                 CompoundId;                                        // 0x0020(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         ProgressRank;                                      // 0x0030(0x0004)(Edit, NativeAccessSpecifierPublic)
	class FName                                   ProgressTrack;                                     // 0x0034(0x0008)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.TrackedObjective
// 0x0030 (0x0030 - 0x0000)
struct FTrackedObjective
{
public:
	class FString                                 UniqueOnlineObjectiveId;                           // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         Target;                                            // 0x0010(0x0004)(Edit, NativeAccessSpecifierPublic)
	class UClass*                                 ObjectiveData;                                     // 0x0018(0x0008)(Edit, NativeAccessSpecifierPublic)
	int32                                         ProgressRank;                                      // 0x0020(0x0004)(Edit, NativeAccessSpecifierPublic)
	class FName                                   ProgressTrack;                                     // 0x0024(0x0008)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.MasteryCardTableData
// 0x0058 (0x0058 - 0x0000)
struct FMasteryCardTableData
{
public:
	TArray<struct FMasteryObjective>              Objectives;                                        // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FDataTableRowHandle>            RewardTracks;                                      // 0x0010(0x0010)(Edit, NativeAccessSpecifierPublic)
	EMasteryCardType                              CardType;                                          // 0x0020(0x0001)(Edit, NativeAccessSpecifierPublic)
	class FString                                 GameObjectId;                                      // 0x0028(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 BaseXpObjectiveId;                                 // 0x0038(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 PrestigeXpObjectiveId;                             // 0x0048(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.AchievementTableData
// 0x0010 (0x0010 - 0x0000)
struct FAchievementTableData
{
public:
	TArray<struct FTrackedObjective>              Objectives;                                        // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
};

