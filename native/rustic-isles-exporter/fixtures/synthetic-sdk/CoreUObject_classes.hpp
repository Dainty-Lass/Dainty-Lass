// Synthetic SDK for the catalog exporter's tests: made up, never generated from a game.

#pragma once

// Class Synthetic.Object
// 0x0028 (0x0028 - 0x0000)
class UObject
{
public:
	void*                                         VTable;                                            // 0x0000(0x0008)(Edit, NativeAccessSpecifierPublic)
	int32                                         Flags;                                             // 0x0008(0x0004)(Edit, NativeAccessSpecifierPublic)
	int32                                         Index;                                             // 0x000C(0x0004)(Edit, NativeAccessSpecifierPublic)
	class UClass*                                 Class;                                             // 0x0010(0x0008)(Edit, NativeAccessSpecifierPublic)
	class FName                                   Name;                                              // 0x0018(0x0008)(Edit, NativeAccessSpecifierPublic)
	class UObject*                                Outer;                                             // 0x0020(0x0008)(Edit, NativeAccessSpecifierPublic)
};

// Class Synthetic.Field
// 0x0008 (0x0030 - 0x0028)
class UField : public UObject
{
public:
	class UField*                                 Next;                                              // 0x0028(0x0008)(Edit, NativeAccessSpecifierPublic)
};

// Class Synthetic.Struct
// 0x0020 (0x0050 - 0x0030)
class UStruct : public UField
{
public:
	uint8                                         Pad_30[0x18];                                      // 0x0030(0x0018)(Edit, NativeAccessSpecifierPublic)
	class UStruct*                                Super;                                             // 0x0048(0x0008)(Edit, NativeAccessSpecifierPublic)
};

// Class Synthetic.Class
// 0x0040 (0x0090 - 0x0050)
class UClass : public UStruct
{
public:
	uint8                                         Pad_50[0x40];                                      // 0x0050(0x0040)(Edit, NativeAccessSpecifierPublic)
};

