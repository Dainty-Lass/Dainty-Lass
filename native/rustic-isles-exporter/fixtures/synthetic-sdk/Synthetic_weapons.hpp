// Synthetic SDK for the catalog exporter's tests: made up, never generated from a game.

#pragma once

// Enum Synthetic.EWeaponPartType
// NumValues: 0x001F
enum class EWeaponPartType : uint8
{
	Weapon_Part_Default                                    = 0,
	Weapon_Part_DP_Barrel                                  = 1,
	Weapon_Part_DP_Receiver                                = 2,
	Weapon_Part_DP_Grip                                    = 3,
	Weapon_Part_DP_Mod                                     = 4,
	Weapon_Part_DP_Prism                                   = 5,
	Weapon_Part_DP_Legendary                               = 6,
	Weapon_Part_EB_Special                                 = 7,
	Weapon_Part_EB_Passive                                 = 8,
	Weapon_Part_EB_Legendary                               = 9,
	Weapon_Part_IH_Special                                 = 10,
	Weapon_Part_IH_Passive                                 = 11,
	Weapon_Part_IH_Legendary                               = 12,
	Weapon_Part_GA_Special                                 = 13,
	Weapon_Part_GA_Passive                                 = 14,
	Weapon_Part_GA_Legendary                               = 15,
	Weapon_Part_CB_Special                                 = 16,
	Weapon_Part_CB_Passive                                 = 17,
	Weapon_Part_CB_Legendary                               = 18,
	Weapon_Part_MS_Special                                 = 19,
	Weapon_Part_MS_Passive                                 = 20,
	Weapon_Part_MS_Legendary                               = 21,
	Weapon_Part_AC_Special                                 = 22,
	Weapon_Part_AC_Passive                                 = 23,
	Weapon_Part_AC_Legendary                               = 24,
	Item_Part_Lantern_Capacitor                            = 30,
	Item_Part_Lantern_Cell                                 = 31,
	Weapon_Part_Soul                                       = 32,
	Weapon_Part_Tracker                                    = 33,
	Weapon_Part_Unused                                     = 34,
	EWeaponPartType_MAX                                    = 35,
};

// ScriptStruct Synthetic.WeaponPartId
// 0x0010 (0x0010 - 0x0000)
struct FWeaponPartId final
{
public:
	class FName                                   ItemId;                                            // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Pad_8[0x8];                                        // 0x0008(0x0008)(Fixing Struct Size After Last Property [ Dumper-7 ])
};

// ScriptStruct Synthetic.ItemPartSlot
// 0x0020 (0x0020 - 0x0000)
struct FItemPartSlot final
{
public:
	EWeaponPartType                               WeaponPartType;                                    // 0x0000(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Pad_1[0x7];                                        // 0x0001(0x0007)(Fixing Size After Last Property [ Dumper-7 ])
	struct FWeaponPartId                          DefaultWeaponPartId;                               // 0x0008(0x0010)(Edit, NativeAccessSpecifierPublic)
	bool                                          bIsLocked;                                         // 0x0018(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bHideFromUI;                                       // 0x0019(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Pad_1A[0x6];                                       // 0x001A(0x0006)(Fixing Struct Size After Last Property [ Dumper-7 ])
};

// ScriptStruct Synthetic.EquippedItemPart
// 0x0018 (0x0018 - 0x0000)
struct FEquippedItemPart final
{
public:
	EWeaponPartType                               WeaponPartType;                                    // 0x0000(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Pad_1[0x7];                                        // 0x0001(0x0007)(Fixing Size After Last Property [ Dumper-7 ])
	struct FWeaponPartId                          WeaponPartID;                                      // 0x0008(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.PlayFabWeaponPartCatalogTableData
// 0x0010 (0x0010 - 0x0000)
struct FPlayFabWeaponPartCatalogTableData final
{
public:
	uint8                                         Pad_0[0x8];                                        // 0x0000(0x0008)(Fixing Size After Last Property [ Dumper-7 ])
	EWeaponPartType                               WeaponPartType;                                    // 0x0008(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bIsUnassignable;                                   // 0x0009(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Pad_A[0x6];                                        // 0x000A(0x0006)(Fixing Struct Size After Last Property [ Dumper-7 ])
};

// Class Synthetic.ArchonInventoryItem
// 0x0020 (0x0048 - 0x0028)
class UArchonInventoryItem : public UObject
{
public:
	class FString                                 ItemId;                                            // 0x0028(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 ItemInstanceID;                                    // 0x0038(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// Class Synthetic.ArchonInventoryItem_Weapon
// 0x0008 (0x0050 - 0x0048)
class UArchonInventoryItem_Weapon final : public UArchonInventoryItem
{
public:
	uint8                                         WeaponType;                                        // 0x0048(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Pad_49[0x7];                                       // 0x0049(0x0007)(Fixing Struct Size After Last Property [ Dumper-7 ])
};

// Class Synthetic.ArchonInventoryItem_WeaponPart
// 0x0010 (0x0058 - 0x0048)
class UArchonInventoryItem_WeaponPart final : public UArchonInventoryItem
{
public:
	uint8                                         WeaponType;                                        // 0x0048(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Pad_49[0x7];                                       // 0x0049(0x0007)(Fixing Size After Last Property [ Dumper-7 ])
	class UDataTable*                             Catalog;                                           // 0x0050(0x0008)(Edit, NativeAccessSpecifierPublic)
};

// Class Synthetic.InventoryItemViewModel
// 0x0038 (0x0060 - 0x0028)
class UInventoryItemViewModel final : public UObject
{
public:
	class FString                                 ItemId;                                            // 0x0028(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 ItemInstanceId;                                    // 0x0038(0x0010)(Edit, NativeAccessSpecifierPublic)
	uint8                                         WeaponType;                                        // 0x0048(0x0001)(Edit, NativeAccessSpecifierPublic)
	EWeaponPartType                               WeaponPartType;                                    // 0x0049(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         bIsEquipped : 1;                                   // 0x004A(0x0001)(BitIndex: 0x00, PropSize: 0x0001 (Edit, NativeAccessSpecifierPublic))
	uint8                                         bHasBeenCrafted : 1;                               // 0x004A(0x0001)(BitIndex: 0x01, PropSize: 0x0001 (Edit, NativeAccessSpecifierPublic))
	uint8                                         bCanBeCrafted : 1;                                 // 0x004A(0x0001)(BitIndex: 0x02, PropSize: 0x0001 (Edit, NativeAccessSpecifierPublic))
	class FName                                   RequiredSlayerNodeId;                              // 0x004C(0x0008)(Edit, NativeAccessSpecifierPublic)
	bool                                          bIsRequiredNodeUnlocked;                           // 0x0054(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Pad_55[0xB];                                       // 0x0055(0x000B)(Fixing Struct Size After Last Property [ Dumper-7 ])
};

// Class Synthetic.ItemPartSlotViewModel
// 0x0008 (0x0030 - 0x0028)
class UItemPartSlotViewModel final : public UObject
{
public:
	bool                                          bIsLocked;                                         // 0x0028(0x0001)(Edit, NativeAccessSpecifierPublic)
	bool                                          bHasPartEquipped;                                  // 0x0029(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Pad_2A[0x2];                                       // 0x002A(0x0002)(Fixing Size After Last Property [ Dumper-7 ])
	int32                                         Rarity;                                            // 0x002C(0x0004)(Edit, NativeAccessSpecifierPublic)
};

// Class Synthetic.LoadoutGearViewModel
// 0x0028 (0x0050 - 0x0028)
class ULoadoutGearViewModel final : public UObject
{
public:
	class UInventoryItemViewModel*                ItemViewModel;                                     // 0x0028(0x0008)(Edit, NativeAccessSpecifierPublic)
	TArray<class UItemPartSlotViewModel*>         PartSlots;                                         // 0x0030(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<class UInventoryItemViewModel*>        PartCandidates;                                    // 0x0040(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// Class Synthetic.LoadoutGearScreen
// 0x0018 (0x0040 - 0x0028)
class ULoadoutGearScreen final : public UObject
{
public:
	class ULoadoutGearViewModel*                  GearViewModel;                                     // 0x0028(0x0008)(Edit, NativeAccessSpecifierPublic)
	class UObject*                                PartSlotBox;                                       // 0x0030(0x0008)(Edit, NativeAccessSpecifierPublic)
	bool                                          bIsAssignPanelOpen;                                // 0x0038(0x0001)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Pad_39[0x7];                                       // 0x0039(0x0007)(Fixing Struct Size After Last Property [ Dumper-7 ])
};

// Function Synthetic.ArchonInventoryItem_Weapon.GetItemPartSlots
// ScriptStruct Synthetic.ArchonInventoryItem_Weapon_GetItemPartSlots
// 0x0010 (0x0010 - 0x0000)
struct ArchonInventoryItem_Weapon_GetItemPartSlots final
{
public:
	TArray<struct FItemPartSlot>                  ReturnValue;                                       // 0x0000(0x0010)(Parm, OutParm, ReturnParm, NativeAccessSpecifierPublic)
};

// Function Synthetic.ArchonInventoryItem_Weapon.GetEquippedItemParts
// ScriptStruct Synthetic.ArchonInventoryItem_Weapon_GetEquippedItemParts
// 0x0010 (0x0010 - 0x0000)
struct ArchonInventoryItem_Weapon_GetEquippedItemParts final
{
public:
	TArray<struct FEquippedItemPart>              ReturnValue;                                       // 0x0000(0x0010)(Parm, OutParm, ReturnParm, NativeAccessSpecifierPublic)
};

// Function Synthetic.ArchonInventoryItem_Weapon.IsItemPartSlotLocked
// ScriptStruct Synthetic.ArchonInventoryItem_Weapon_IsItemPartSlotLocked
// 0x0002 (0x0002 - 0x0000)
struct ArchonInventoryItem_Weapon_IsItemPartSlotLocked final
{
public:
	EWeaponPartType                               PartType;                                          // 0x0000(0x0001)(Parm, ZeroConstructor, IsPlainOldData, NoDestructor, NativeAccessSpecifierPublic)
	bool                                          ReturnValue;                                       // 0x0001(0x0001)(Parm, OutParm, ZeroConstructor, ReturnParm, IsPlainOldData, NoDestructor, NativeAccessSpecifierPublic)
};
