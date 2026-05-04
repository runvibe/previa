import { Trash2, PanelRightOpen, PanelRightClose, PanelBottomOpen, PanelBottomClose } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";

interface RunHistoryPanelProps {
  title?: string;
  onClear?: () => void;
  isEmpty: boolean;
  children: React.ReactNode;
  onCollapse?: () => void;
  collapsed?: boolean;
  collapseDirection?: "side" | "bottom";
  collapseOnHeaderClick?: boolean;
}

export function RunHistoryPanel({ title, onClear, isEmpty, children, onCollapse, collapsed, collapseDirection = "side", collapseOnHeaderClick = false }: RunHistoryPanelProps) {
  const { t } = useTranslation();
  const displayTitle = title ?? t("history.title");
  const CollapseIcon = collapseDirection === "bottom"
    ? (collapsed ? PanelBottomOpen : PanelBottomClose)
    : (collapsed ? PanelRightOpen : PanelRightClose);
  const handleHeaderKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!collapseOnHeaderClick || !onCollapse) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onCollapse();
    }
  };
  if (isEmpty) return null;

  return (
    <div className="glass flex flex-col h-full">
      <div
        className={cn(
          "flex items-center justify-between border-border/50 px-3 py-2",
          collapseOnHeaderClick && onCollapse && "cursor-pointer select-none",
        )}
        onClick={collapseOnHeaderClick ? onCollapse : undefined}
        onKeyDown={handleHeaderKeyDown}
        role={collapseOnHeaderClick && onCollapse ? "button" : undefined}
        tabIndex={collapseOnHeaderClick && onCollapse ? 0 : undefined}
      >
        <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{displayTitle}</p>
        <div className="flex items-center gap-0.5">
          {onClear && (
            <Button
              variant="ghost"
              size="icon"
              className="h-5 w-5 text-muted-foreground hover:text-destructive"
              onClick={(event) => {
                event.stopPropagation();
                onClear();
              }}
            >
              <Trash2 className="h-3 w-3" />
            </Button>
          )}
          {onCollapse && (
            <Button
              variant="ghost"
              size="icon"
              className="h-5 w-5 text-muted-foreground hover:text-foreground"
              onClick={(event) => {
                event.stopPropagation();
                onCollapse();
              }}
              title={collapsed ? "Expand history" : "Collapse history"}
            >
              <CollapseIcon className="h-3 w-3" />
            </Button>
          )}
        </div>
      </div>
      <ScrollArea className="flex-1">
        <div className="flex flex-col gap-0">
          {children}
        </div>
      </ScrollArea>
    </div>
  );
}
