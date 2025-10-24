import React from "react";

interface SnareTabProps {
  isLoaded: boolean;
  isPlaying: boolean;
  snarePreset: string;
  snareConfig: {
    frequency: number;
    tonal: number;
    noise: number;
    crack: number;
    decay: number;
    pitchDrop: number;
    volume: number;
  };
  triggerSnareDrum: () => void;
  releaseSnareDrum: () => void;
  handleSnareConfigChange: (param: string, value: number) => void;
  handleSnarePresetChange: (preset: string) => void;
}

export default function SnareTab({
  isLoaded,
  isPlaying,
  snarePreset,
  snareConfig,
  triggerSnareDrum,
  releaseSnareDrum,
  handleSnareConfigChange,
  handleSnarePresetChange,
}: SnareTabProps) {
  return (
    <div>
      <h3 className="font-semibold mb-4 text-center text-lg">🥁 Snare Drum</h3>

      {/* Snare Drum Trigger Button */}
      <button
        onClick={triggerSnareDrum}
        disabled={!isLoaded || !isPlaying}
        className="w-full px-4 py-3 mb-4 bg-gradient-to-r from-orange-600 to-orange-700 text-white rounded-lg hover:from-orange-700 hover:to-orange-800 disabled:bg-gray-400 disabled:cursor-not-allowed font-semibold text-lg shadow-lg"
      >
        🥁 TRIGGER SNARE
      </button>

      {/* Preset Selection */}
      <div className="mb-4">
        <label className="block text-sm font-medium mb-2">Preset</label>
        <select
          value={snarePreset}
          onChange={(e) => handleSnarePresetChange(e.target.value)}
          disabled={!isLoaded}
          className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded disabled:cursor-not-allowed"
        >
          <option value="default">Default</option>
          <option value="crispy">Crispy</option>
          <option value="deep">Deep</option>
          <option value="tight">Tight</option>
          <option value="fat">Fat</option>
        </select>
      </div>

      {/* Snare Drum Controls */}
      <div className="space-y-3">
        {/* Frequency */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Frequency</label>
          <input
            type="range"
            min="100"
            max="600"
            step="1"
            value={snareConfig.frequency}
            onChange={(e) =>
              handleSnareConfigChange("frequency", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {snareConfig.frequency.toFixed(0)}Hz
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
            value={snareConfig.volume}
            onChange={(e) =>
              handleSnareConfigChange("volume", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {snareConfig.volume.toFixed(2)}
          </span>
        </div>

        {/* Decay Time */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Decay</label>
          <input
            type="range"
            min="0.01"
            max="2"
            step="0.01"
            value={snareConfig.decay}
            onChange={(e) =>
              handleSnareConfigChange("decay", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {snareConfig.decay.toFixed(2)}s
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
            value={snareConfig.tonal}
            onChange={(e) =>
              handleSnareConfigChange("tonal", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {snareConfig.tonal.toFixed(2)}
          </span>
        </div>

        {/* Noise Amount */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Noise</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={snareConfig.noise}
            onChange={(e) =>
              handleSnareConfigChange("noise", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {snareConfig.noise.toFixed(2)}
          </span>
        </div>

        {/* Crack Amount */}
        <div className="flex items-center space-x-2">
          <label className="w-20 text-sm font-medium">Crack</label>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={snareConfig.crack}
            onChange={(e) =>
              handleSnareConfigChange("crack", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {snareConfig.crack.toFixed(2)}
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
            value={snareConfig.pitchDrop}
            onChange={(e) =>
              handleSnareConfigChange("pitchDrop", parseFloat(e.target.value))
            }
            disabled={!isLoaded}
            className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
          />
          <span className="w-12 text-sm font-mono text-right">
            {snareConfig.pitchDrop.toFixed(2)}
          </span>
        </div>
      </div>

      {/* Release Button */}
      <button
        onClick={releaseSnareDrum}
        disabled={!isLoaded || !isPlaying}
        className="w-full mt-4 px-4 py-2 bg-gray-600 text-white rounded hover:bg-gray-500 disabled:bg-gray-700 disabled:cursor-not-allowed"
      >
        Release Snare
      </button>
    </div>
  );
}