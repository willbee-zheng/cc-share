import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Calculator } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { fetchPricing, type PricingEntry } from "@/lib/api";

const SUPPLIER_RATE = 0.9;

/** Hardcoded defaults used when cloud pricing is unavailable. */
const DEFAULT_PRICING: PricingEntry[] = [
  { model_prefix: "claude-opus-4", prompt_per_1k: 15, completion_per_1k: 75 },
  { model_prefix: "claude-sonnet-4", prompt_per_1k: 3, completion_per_1k: 15 },
  { model_prefix: "claude-haiku-4", prompt_per_1k: 0.8, completion_per_1k: 4 },
  { model_prefix: "claude-3-5-sonnet", prompt_per_1k: 3, completion_per_1k: 15 },
  { model_prefix: "gpt-4o", prompt_per_1k: 2.5, completion_per_1k: 10 },
  { model_prefix: "gpt-4o-mini", prompt_per_1k: 0.15, completion_per_1k: 0.6 },
  { model_prefix: "gpt-4", prompt_per_1k: 10, completion_per_1k: 30 },
  { model_prefix: "gemini-1.5-pro", prompt_per_1k: 1.25, completion_per_1k: 5 },
  { model_prefix: "gemini-1.5-flash", prompt_per_1k: 0.075, completion_per_1k: 0.3 },
  { model_prefix: "deepseek", prompt_per_1k: 0.14, completion_per_1k: 0.28 },
];

interface ModelPreset {
  id: string;
  label: string;
  /** Blended rate = (prompt + completion) / 2 per 1K tokens */
  pricePerKToken: number;
}

function pricingToPresets(entries: PricingEntry[]): ModelPreset[] {
  return entries.map((e) => ({
    id: e.model_prefix,
    label: e.model_prefix,
    pricePerKToken: (e.prompt_per_1k + e.completion_per_1k) / 2,
  }));
}

/**
 * 收益预估计算器：输入活跃小时、平均 tokens/小时、模型，输出每日/每月预估积分。
 *
 * 公式：daily = activeHours × tokensPerHour × pricePerKToken/1000 × supplierRate
 *      monthly = daily × 30
 */
export function EarningsCalculator() {
  const { t } = useTranslation("share");
  const [activeHours, setActiveHours] = useState(8);
  const [tokensPerHour, setTokensPerHour] = useState(50_000);
  const [models, setModels] = useState<ModelPreset[]>(() =>
    pricingToPresets(DEFAULT_PRICING),
  );
  const [modelId, setModelId] = useState(models[0]?.id ?? "claude-sonnet-4");
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchPricing()
      .then((entries) => {
        if (entries.length > 0) {
          setModels(pricingToPresets(entries));
          setModelId(entries[0].model_prefix);
        }
      })
      .catch(() => {
        // Fall back to defaults on error
      })
      .finally(() => setLoading(false));
  }, []);

  const model = models.find((m) => m.id === modelId) ?? models[0];
  const dailyTokens = activeHours * tokensPerHour;
  const dailyCredits = (dailyTokens / 1000) * model.pricePerKToken * SUPPLIER_RATE;
  const monthlyCredits = dailyCredits * 30;

  if (loading) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <Calculator className="w-4 h-4" />
            {t("calculator.title")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">{t("calculator.loading") ?? "Loading pricing..."}</p>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <Calculator className="w-4 h-4" />
          {t("calculator.title")}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1.5">
            <Label htmlFor="calc-hours">{t("calculator.activeHours")}</Label>
            <Input
              id="calc-hours"
              type="number"
              min={0}
              max={24}
              value={activeHours}
              onChange={(e) => setActiveHours(Math.max(0, Math.min(24, Number(e.target.value) || 0)))}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="calc-tokens">{t("calculator.tokensPerHour")}</Label>
            <Input
              id="calc-tokens"
              type="number"
              min={0}
              step={5000}
              value={tokensPerHour}
              onChange={(e) => setTokensPerHour(Math.max(0, Number(e.target.value) || 0))}
            />
          </div>
        </div>

        <div className="space-y-1.5">
          <Label htmlFor="calc-model">{t("calculator.model")}</Label>
          <select
            id="calc-model"
            value={modelId}
            onChange={(e) => setModelId(e.target.value)}
            className="w-full h-9 rounded-md border bg-background px-3 text-sm"
          >
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label} ({m.pricePerKToken.toFixed(2)}/1K)
              </option>
            ))}
          </select>
        </div>

        <div className="grid grid-cols-2 gap-3 pt-2">
          <div className="rounded-lg bg-muted/50 p-3 text-center">
            <p className="text-xs text-muted-foreground">{t("calculator.daily")}</p>
            <p className="text-xl font-bold text-emerald-600">
              +{dailyCredits.toFixed(2)}
            </p>
          </div>
          <div className="rounded-lg bg-muted/50 p-3 text-center">
            <p className="text-xs text-muted-foreground">{t("calculator.monthly")}</p>
            <p className="text-xl font-bold text-emerald-600">
              +{monthlyCredits.toFixed(0)}
            </p>
          </div>
        </div>
        <p className="text-xs text-muted-foreground">
          {t("calculator.disclaimer")}
        </p>
      </CardContent>
    </Card>
  );
}