//! The scene's enums: `ShakeType` (`Tactical.getShaker().shake`), `CloudType` (cloud
//! settings' `Type`) and `AmbientLightingPreset` (`setAmbientLightingPreset`).

use super::script_enum;

script_enum! {
	/// How a unit shakes (see `tactical_entities::shaker`).
	ShakeType {
		None = 0,
		Jump = 1,
		KnockBack = 2,
		LightKnockBack = 3,
		Dodge = 4,
		Attack = 5,
		LightAttack = 6,
		KnockUp = 7,
	}
}

script_enum! {
	/// What kind of clouds a cloud cover builds (see `world_clouds`).
	CloudType {
		Default = 0,
		Fog = 1,
		StaticFog = 2,
		SunHighlights = 3,
		Custom = 4,
	}
}

script_enum! {
	/// A battle's ambient light (see `tactical`'s lighting presets).
	AmbientLightingPreset {
		Default = 0,
		Dawn = 1,
		Dusk = 2,
		Night = 3,
		Midday = 4,
		Rain = 5,
		Storm = 6,
		Desert = 7,
	}
}
