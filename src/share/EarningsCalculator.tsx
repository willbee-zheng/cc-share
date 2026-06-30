import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Calculator } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface ModelPreset {
  id: string;
  /** credits per 1K prompt+completion combined (rough avg) */
  pricePerKToken: number;
}

const MODELS: ModelPreset[] = [
  { id: "claude-sonnet-4-6", pricePerKToken: 9 },
  { id: "claude-opus-4-8", pricePerKToken: 45 },
  { id: "claude-haiku-4-5", pricePerKToken: 2.4 },
  { id: "gpt-4o", pricePerKToken: 6.25 },
  { id: "gpt-4o-mini", pricePerKToken: 0.375 },
  { id: "gemini-1.5-pro", pricePerKToken: 3.125 },
  { id: "deepseek", pricePerKToken: 0.21 },
];

const SUPPLIER_RATE = 0.9;

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
  const [modelId, setModelId] = useState(MODELS[0].id);

  const model = MODELS.find((m) => m.id === modelId) ?? MODELS[0];
  const dailyTokens = activeHours * tokensPerHour;
  const dailyCredits = (dailyTokens / 1000) * model.pricePerKToken * SUPPLIER_RATE;
  const monthlyCredits = dailyCredits * 30;

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
            {MODELS.map((m) => (
              <option key={m.id} value={m.id}>
                {m.id} ({m.pricePerKToken}/1K)
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
