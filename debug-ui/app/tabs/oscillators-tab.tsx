import React from "react";

interface OscillatorsTabProps {
  isLoaded: boolean;
  isPlaying: boolean;
  volumes: number[];
  frequencies: number[];
  modulatorFrequencies: number[];
  waveforms: number[];
  enabled: boolean[];
  adsrValues: Array<{
    attack: number;
    decay: number;
    sustain: number;
    release: number;
  }>;
  triggerAll: () => void;
  triggerInstrument: (index: number, name: string) => void;
  handleVolumeChange: (index: number, volume: number) => void;
  handleFrequencyChange: (index: number, frequency: number) => void;
  handleWaveformChange: (index: number, waveformType: number) => void;
  handleAdsrChange: (
    index: number,
    param: "attack" | "decay" | "sustain" | "release",
    value: number
  ) => void;
  handleModulatorFrequencyChange: (index: number, frequency: number) => void;
  handleEnabledChange: (index: number, isEnabled: boolean) => void;
  releaseInstrument: (index: number, name: string) => void;
  releaseAll: () => void;
}

export default function OscillatorsTab({
  isLoaded,
  isPlaying,
  volumes,
  frequencies,
  modulatorFrequencies,
  waveforms,
  enabled,
  adsrValues,
  triggerAll,
  triggerInstrument,
  handleVolumeChange,
  handleFrequencyChange,
  handleWaveformChange,
  handleAdsrChange,
  handleModulatorFrequencyChange,
  handleEnabledChange,
  releaseInstrument,
  releaseAll,
}: OscillatorsTabProps) {
  return (
    <div>
      <h3 className="font-semibold mb-3 text-center">Instrument Controls</h3>

      <button
        onClick={triggerAll}
        disabled={!isLoaded || !isPlaying}
        className="w-full px-4 py-2 mb-3 bg-purple-500 text-white rounded hover:bg-purple-600 disabled:bg-gray-400 disabled:cursor-not-allowed font-semibold"
      >
        🥁 Trigger All Instruments
      </button>

      <div className="grid grid-cols-2 gap-2">
        <button
          onClick={() => triggerInstrument(0, "Bass Drum")}
          disabled={!isLoaded || !isPlaying}
          className="px-3 py-2 bg-red-500 text-white rounded hover:bg-red-600 disabled:bg-gray-400 disabled:cursor-not-allowed text-sm"
        >
          🥁 Bass (200Hz)
        </button>

        <button
          onClick={() => triggerInstrument(1, "Snare")}
          disabled={!isLoaded || !isPlaying}
          className="px-3 py-2 bg-orange-500 text-white rounded hover:bg-orange-600 disabled:bg-gray-400 disabled:cursor-not-allowed text-sm"
        >
          🥁 Snare (300Hz)
        </button>

        <button
          onClick={() => triggerInstrument(2, "Hi-hat")}
          disabled={!isLoaded || !isPlaying}
          className="px-3 py-2 bg-yellow-500 text-white rounded hover:bg-yellow-600 disabled:bg-gray-400 disabled:cursor-not-allowed text-sm"
        >
          🔔 Hi-hat (440Hz)
        </button>

        <button
          onClick={() => triggerInstrument(3, "Cymbal")}
          disabled={!isLoaded || !isPlaying}
          className="px-3 py-2 bg-cyan-500 text-white rounded hover:bg-cyan-600 disabled:bg-gray-400 disabled:cursor-not-allowed text-sm"
        >
          🥽 Cymbal (600Hz)
        </button>
      </div>

      <div className="mt-6">
        <h4 className="font-semibold mb-3 text-center">Instrument Controls</h4>
        <div className="space-y-4">
          {[
            { name: "🥁 Bass Drum", color: "red" },
            { name: "🥁 Snare", color: "orange" },
            { name: "🔔 Hi-hat", color: "yellow" },
            { name: "🥽 Cymbal", color: "cyan" },
          ].map((instrument, index) => (
            <div
              key={index}
              className="p-3 bg-gray-800 rounded-lg border border-gray-700"
            >
              <div className="flex items-center justify-between mb-2">
                <h5 className="font-medium text-sm">{instrument.name}</h5>
                <button
                  onClick={() => handleEnabledChange(index, !enabled[index])}
                  disabled={!isLoaded}
                  className={`px-3 py-1 text-xs font-medium rounded transition-colors disabled:cursor-not-allowed ${
                    enabled[index]
                      ? "bg-green-600 text-white hover:bg-green-700 disabled:bg-gray-600"
                      : "bg-red-600 text-white hover:bg-red-700 disabled:bg-gray-600"
                  }`}
                >
                  {enabled[index] ? "🔊 ON" : "🔇 OFF"}
                </button>
              </div>

              {/* Volume Control */}
              <div className="flex items-center space-x-2 mb-2">
                <label className="w-12 text-xs font-medium">Volume</label>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.01"
                  value={volumes[index]}
                  onChange={(e) =>
                    handleVolumeChange(index, parseFloat(e.target.value))
                  }
                  disabled={!isLoaded}
                  className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
                />
                <span className="w-10 text-xs font-mono text-right">
                  {volumes[index].toFixed(2)}
                </span>
              </div>

              {/* Frequency Control */}
              <div className="flex items-center space-x-2 mb-2">
                <label className="w-12 text-xs font-medium">Freq</label>
                <input
                  type="range"
                  min="50"
                  max="2000"
                  step="10"
                  value={frequencies[index]}
                  onChange={(e) =>
                    handleFrequencyChange(index, parseInt(e.target.value))
                  }
                  disabled={!isLoaded}
                  className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
                />
                <span className="w-16 text-xs font-mono text-right">
                  {frequencies[index]}Hz
                </span>
              </div>

              {/* Waveform Control */}
              <div className="flex items-center space-x-2 mb-3">
                <label className="w-12 text-xs font-medium">Wave</label>
                <select
                  value={waveforms[index]}
                  onChange={(e) =>
                    handleWaveformChange(index, parseInt(e.target.value))
                  }
                  disabled={!isLoaded}
                  className="flex-1 px-2 py-1 text-xs bg-gray-700 border border-gray-600 rounded disabled:cursor-not-allowed"
                >
                  <option value={0}>Sine</option>
                  <option value={1}>Square</option>
                  <option value={2}>Saw</option>
                  <option value={3}>Triangle</option>
                  <option value={4}>Ring Mod</option>
                </select>
              </div>

              {/* Modulator Frequency Control (only for Ring Mod) */}
              {waveforms[index] === 4 && (
                <div className="flex items-center space-x-2 mb-3">
                  <label className="w-12 text-xs font-medium">Mod</label>
                  <input
                    type="range"
                    min="50"
                    max="2000"
                    step="10"
                    value={modulatorFrequencies[index]}
                    onChange={(e) =>
                      handleModulatorFrequencyChange(
                        index,
                        parseInt(e.target.value)
                      )
                    }
                    disabled={!isLoaded}
                    className="flex-1 h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
                  />
                  <span className="w-16 text-xs font-mono text-right">
                    {modulatorFrequencies[index]}Hz
                  </span>
                </div>
              )}

              {/* ADSR Controls */}
              <div className="border-t border-gray-600 pt-2">
                <div className="flex items-center justify-between mb-2">
                  <label className="text-xs font-medium text-gray-300">
                    ADSR Envelope
                  </label>
                  <button
                    onClick={() =>
                      releaseInstrument(
                        index,
                        instrument.name.split(" ")[1] || instrument.name
                      )
                    }
                    disabled={!isLoaded || !isPlaying}
                    className="px-2 py-1 text-xs bg-gray-600 text-white rounded hover:bg-gray-500 disabled:bg-gray-700 disabled:cursor-not-allowed"
                  >
                    Release
                  </button>
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <label className="block text-xs text-gray-400 mb-1">
                      Attack
                    </label>
                    <div className="flex items-center space-x-2">
                      <input
                        type="range"
                        min="0.001"
                        max="2"
                        step="0.001"
                        value={adsrValues[index].attack}
                        onChange={(e) =>
                          handleAdsrChange(
                            index,
                            "attack",
                            parseFloat(e.target.value)
                          )
                        }
                        disabled={!isLoaded}
                        className="flex-1 h-1 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
                      />
                      <span className="w-12 text-xs font-mono text-right">
                        {adsrValues[index].attack.toFixed(3)}s
                      </span>
                    </div>
                  </div>
                  <div>
                    <label className="block text-xs text-gray-400 mb-1">
                      Decay
                    </label>
                    <div className="flex items-center space-x-2">
                      <input
                        type="range"
                        min="0.001"
                        max="2"
                        step="0.001"
                        value={adsrValues[index].decay}
                        onChange={(e) =>
                          handleAdsrChange(
                            index,
                            "decay",
                            parseFloat(e.target.value)
                          )
                        }
                        disabled={!isLoaded}
                        className="flex-1 h-1 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
                      />
                      <span className="w-12 text-xs font-mono text-right">
                        {adsrValues[index].decay.toFixed(3)}s
                      </span>
                    </div>
                  </div>
                  <div>
                    <label className="block text-xs text-gray-400 mb-1">
                      Sustain
                    </label>
                    <div className="flex items-center space-x-2">
                      <input
                        type="range"
                        min="0"
                        max="1"
                        step="0.01"
                        value={adsrValues[index].sustain}
                        onChange={(e) =>
                          handleAdsrChange(
                            index,
                            "sustain",
                            parseFloat(e.target.value)
                          )
                        }
                        disabled={!isLoaded}
                        className="flex-1 h-1 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
                      />
                      <span className="w-12 text-xs font-mono text-right">
                        {adsrValues[index].sustain.toFixed(2)}
                      </span>
                    </div>
                  </div>
                  <div>
                    <label className="block text-xs text-gray-400 mb-1">
                      Release
                    </label>
                    <div className="flex items-center space-x-2">
                      <input
                        type="range"
                        min="0.001"
                        max="5"
                        step="0.001"
                        value={adsrValues[index].release}
                        onChange={(e) =>
                          handleAdsrChange(
                            index,
                            "release",
                            parseFloat(e.target.value)
                          )
                        }
                        disabled={!isLoaded}
                        className="flex-1 h-1 bg-gray-200 rounded-lg appearance-none cursor-pointer disabled:cursor-not-allowed"
                      />
                      <span className="w-12 text-xs font-mono text-right">
                        {adsrValues[index].release.toFixed(3)}s
                      </span>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>

      <button
        onClick={releaseAll}
        disabled={!isLoaded || !isPlaying}
        className="w-full mt-3 px-4 py-2 bg-gray-600 text-white rounded hover:bg-gray-500 disabled:bg-gray-700 disabled:cursor-not-allowed"
      >
        Release All Instruments
      </button>
    </div>
  );
}