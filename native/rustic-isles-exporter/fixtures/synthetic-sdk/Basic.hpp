// Synthetic SDK for the catalog exporter's tests: made up, never generated from a game.

#pragma once

namespace Offsets
{
	constexpr int32 GObjects          = 0x00123400;
	constexpr int32 AppendString      = 0x00045600;
	constexpr int32 GNames            = 0x00234500;
	constexpr int32 ProcessEvent      = 0x00056700;
	constexpr int32 ProcessEventIdx   = 0x00000021;
}

class TUObjectArray
{
public:
	enum
	{
		ElementsPerChunk = 0x400,
	};
};

// Class Synthetic.TextData
// 0x0030 (0x0030 - 0x0000)
class FTextData
{
public:
	uint8                                         Pad_0[0x20];                                       // 0x0000(0x0020)(Edit, NativeAccessSpecifierPublic)
	class FString                                 TextSource;                                        // 0x0020(0x0010)(Edit, NativeAccessSpecifierPublic)
};

