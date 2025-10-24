import React from "react";

interface TomTabProps {
  isLoaded: boolean;
  isPlaying: boolean;
  tomPreset: string;
  tomConfig: {
    frequency: number;
    tonal: number;
    punch: number;
    decay: number;
    pitchDrop: number;
    volume: number;
  };
  triggerTomDrum: () => void;
  releaseTomDrum: () => void;
  handleTomConfigChange: (param: string, value: number) => void;
  handleTomPresetChange: (preset: string) => void;
}

export default function TomTab({
  isLoaded,
  isPlaying,
  tomPreset,
  tomConfig,
  triggerTomDrum,
  releaseTomDrum,
  handleTomConfigChange,
  handleTomPresetChange,
}: TomTabProps) {
  return (
    <div>
      <h3 className="font-semibold mb-4 text-center text-lg">🥁 Tom Drum</h3>

      {/* Tom Drum Trigger Button */}
      <button
        onClick={triggerTomDrum}
        disabled={!isLoaded || !isPlaying}
        className="w-full px-4 py-3 mb-4 bg-gradient-to-r from-purple-600 to-purple-700 text-white rounded-lg hover:from-purple-700 hover:to-purple-800 disabled:bg-gray-400 disabled:cursor-not-allowed font-semibold text-lg shadow-lg"
      >
        🥁 TRIGGER TOM
      </button>

      {/* Preset Selection */}
      <div className="mb-4">
        <label className="block text-sm font-medium mb-2">Preset</label>
        <select
          value={tomPreset}
          onChange={(e) => handleTomPresetChange(e.target.value)}
          disabled={!isLoaded}
          className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded disabled:cursor-not-allowed"
        >
          <option value="default">Default</option>
          <option value="high_tom">High Tom</option>
          <option value="mid_tom">Mid Tom</option>
          <option value="low_tom">Low Tom</option>
          <option value="floor_tom">Floor Tom</option>
        </select>
      </div>

      {/* Tom Drum Controls */}
      <div className="space-y-3">
        {/* Frequency */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Frequency</label>
          <input
            type="range"
            min="60"
            max="400"
            step="1"
            value={tomConfig.frequency}
            onChange={(e) =>
              handleTomConfigChange("frequency", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {tomConfig.frequency.toFixed(0)}Hz
          </span>
        </div>

        {/* Volume */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Volume</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={tomConfig.volume}
            onChange={(e) =>
              handleTomConfigChange("volume", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {tomConfig.volume.toFixed(2)}
          </span>
        </div>

        {/* Decay Time */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Decay</label>
          <input
            type="range"
            min="0.05"
            max="3"
            step="0.01"
            value={tomConfig.decay}
            onChange={(e) =>
              handleTomConfigChange("decay", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {tomConfig.decay.toFixed(2)}s
          </span>
        </div>

        {/* Tonal Amount */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Tonal</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={tomConfig.tonal}
            onChange={(e) =>
              handleTomConfigChange("tonal", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {tomConfig.tonal.toFixed(2)}
          </span>
        </div>

        {/* Punch Amount */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Punch</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={tomConfig.punch}
            onChange={(e) =>
              handleTomConfigChange("punch", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {tomConfig.punch.toFixed(2)}
          </span>
        </div>

        {/* Pitch Drop */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Pitch Drop</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={tomConfig.pitchDrop}
            onChange={(e) =>
              handleTomConfigChange("pitchDrop", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {tomConfig.pitchDrop.toFixed(2)}
          </span>
        </div>
      </div>

      {/* Release Button */}
      <button
        onClick={releaseTomDrum}
        disabled={!isLoaded || !isPlaying}
        className="w-full mt-4 px-4 py-2 bg-gray-600 text-white rounded hover:bg-gray-500 disabled:bg-gray-700 disabled:cursor-not-allowed"
      >
        Release Tom
      </button>
    </div>
  );
}