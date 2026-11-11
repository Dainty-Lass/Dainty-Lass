// Synthetic SDK for the catalog exporter's tests: made up, never generated from a game.

#pragma once

// Class Synthetic.StoreViewModel
// 0x0040 (0x0068 - 0x0028)
class UStoreViewModel : public UObject
{
public:
	uint8                                         Pad_28[0x10];                                      // 0x0028(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 TagFilter;                                         // 0x0038(0x0010)(Edit, NativeAccessSpecifierPublic)
	class UDataTable*                             StoreItemsTable;                                   // 0x0048(0x0008)(Edit, NativeAccessSpecifierPublic)
	class UDataTable*                             CurrencyDataTable;                                 // 0x0050(0x0008)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FStoreCategoryViewModels>       StoreCategoryViewModels;                           // 0x0058(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// Class Synthetic.StoreItemViewModel
// 0x01B0 (0x01D8 - 0x0028)
class UStoreItemViewModel : public UObject
{
public:
	class FText                                   DisplayName;                                       // 0x0028(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FText                                   DisplayDescription;                                // 0x0040(0x0018)(Edit, NativeAccessSpecifierPublic)
	class FText                                   DisplayItemType;                                   // 0x0058(0x0018)(Edit, NativeAccessSpecifierPublic)
	int32                                         ItemPriceNumber;                                   // 0x0070(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         ItemCurrentPriceNumber;                            // 0x0074(0x0004)(Edit, NativeAccessSpecifierPublic)
	uint8                                         StoreCurrency;                                     // 0x0078(0x0001)(Edit, NativeAccessSpecifierPublic)
	class FString                                 CurrencyCode;                                      // 0x0080(0x0010)(Edit, NativeAccessSpecifierPublic)
	int32                                         DisplayPriority;                                   // 0x0090(0x0004)(Edit, NativeAccessSpecifierPublic)
	bool                                          bIsPurchased;                                      // 0x0094(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bCanPurchaseAgain;                                 // 0x0095(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bIsExpired;                                        // 0x0096(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bIsPrestigeItemElite;                              // 0x0097(0x0001)(Edit, NativeAccessSpecifierPublic)
	int32                                         RewardRarityLevel;                                 // 0x0098(0x0004)(Edit, NativeAccessSpecifierPublic)
	struct FStoreItemCatalogData                  StoreItemCatalogData;                              // 0x00A0(0x0010)(Edit, NativeAccessSpecifierPublic)
	struct FOnlineStorePhoenixOffer               StoreItem;                                         // 0x00B0(0x0128)(Edit, NativeAccessSpecifierPublic)
};

// Class Synthetic.PlayerExperienceComponent
// 0x0020 (0x0048 - 0x0028)
class UPlayerExperienceComponent : public UObject
{
public:
	uint8                                         Pad_28[0x18];                                      // 0x0028(0x0018)(Edit, NativeAccessSpecifierPublic)
	class UDataTable*                             ExperienceTable;                                   // 0x0040(0x0008)(Edit, NativeAccessSpecifierPublic)
};

