// Synthetic SDK for the catalog exporter's tests: made up, never generated from a game.

#pragma once

// ScriptStruct Synthetic.GameplayTag
// 0x0008 (0x0008 - 0x0000)
struct FGameplayTag
{
public:
	class FName                                   TagName;                                           // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.GameplayTagContainer
// 0x0020 (0x0020 - 0x0000)
struct FGameplayTagContainer
{
public:
	TArray<struct FGameplayTag>                   GameplayTags;                                      // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FGameplayTag>                   ParentTags;                                        // 0x0010(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.GameplayTagQuery
// 0x0048 (0x0048 - 0x0000)
struct FGameplayTagQuery
{
public:
	int32                                         TokenStreamVersion;                                // 0x0000(0x0004)(Edit, NativeAccessSpecifierPublic)
	TArray<struct FGameplayTag>                   TagDictionary;                                     // 0x0008(0x0010)(Edit, NativeAccessSpecifierPublic)
	TArray<uint8>                                 QueryTokenStream;                                  // 0x0018(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 UserDescription;                                   // 0x0028(0x0010)(Edit, NativeAccessSpecifierPublic)
	class FString                                 AutoDescription;                                   // 0x0038(0x0010)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.DataTableRowHandle
// 0x0010 (0x0010 - 0x0000)
struct FDataTableRowHandle
{
public:
	class UDataTable*                             DataTable;                                         // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	class FName                                   RowName;                                           // 0x0008(0x0008)(Edit, NativeAccessSpecifierPublic)
};

// ScriptStruct Synthetic.DateTime
// 0x0008 (0x0008 - 0x0000)
struct FDateTime
{
public:
	uint8                                         Pad_0[0x8];                                        // 0x0000(0x0008)(Fixing Struct Size After Last Property [ Dumper-7 ])
};

// ScriptStruct Synthetic.GameplayAttribute
// 0x0038 (0x0038 - 0x0000)
struct FGameplayAttribute
{
public:
	class FString                                 AttributeName;                                     // 0x0000(0x0010)(Edit, NativeAccessSpecifierPublic)
	uint8                                         Pad_10[0x20];                                      // 0x0010(0x0020)(Edit, NativeAccessSpecifierPublic)
	class UStruct*                                AttributeOwner;                                    // 0x0030(0x0008)(Edit, NativeAccessSpecifierPublic)
};

// Class Synthetic.DataTable
// 0x0058 (0x0080 - 0x0028)
class UDataTable : public UObject
{
public:
	class UScriptStruct*                          RowStruct;                                         // 0x0028(0x0008)(Edit, NativeAccessSpecifierPublic)
	TMap<class FName, uint8*>                     RowMap;                                            // 0x0030(0x0050)(Edit, NativeAccessSpecifierPublic)
};

