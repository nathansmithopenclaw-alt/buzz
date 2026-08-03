import * as React from "react";
import { ChevronDown } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  type PersonaDropdownOption,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";

export function PersonaDropdownField({
  contentClassName,
  disabled,
  id,
  onValueChange,
  options,
  placeholder,
  value,
}: {
  contentClassName?: string;
  disabled?: boolean;
  id: string;
  onValueChange: (value: string) => void;
  options: readonly PersonaDropdownOption[];
  placeholder: string;
  value: string;
}) {
  const [open, setOpen] = React.useState(false);
  const selectedOption = options.find((option) => option.value === value);

  return (
    <div className={PERSONA_FIELD_SHELL_CLASS}>
      <DropdownMenu modal={false} onOpenChange={setOpen} open={open}>
        <DropdownMenuTrigger asChild>
          <button
            className={cn(
              "flex h-11 w-full items-center justify-between gap-3 px-3 py-2 text-left text-sm leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
              disabled && "cursor-default opacity-60",
            )}
            disabled={disabled}
            id={id}
            type="button"
          >
            <span
              className={cn(
                "min-w-0 flex-1 truncate",
                !selectedOption && "text-muted-foreground/55",
              )}
            >
              {selectedOption?.label ?? placeholder}
            </span>
            <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground/60" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="start"
          className={cn("overflow-hidden", contentClassName)}
          onCloseAutoFocus={(event) => event.preventDefault()}
          sideOffset={5}
          style={{
            minWidth: "var(--radix-dropdown-menu-trigger-width)",
            width: "var(--radix-dropdown-menu-trigger-width)",
          }}
        >
          <div
            className="max-h-[min(16rem,var(--radix-dropdown-menu-content-available-height))] overflow-y-auto overscroll-contain"
            onTouchMoveCapture={(event) => event.stopPropagation()}
            onWheelCapture={(event) => event.stopPropagation()}
          >
            <DropdownMenuRadioGroup
              onValueChange={(nextValue) => {
                onValueChange(nextValue);
                setOpen(false);
              }}
              value={value}
            >
              {options.map((option, index) => (
                <React.Fragment key={option.value}>
                  {option.group &&
                  option.group !== options[index - 1]?.group ? (
                    <div className="px-2 pb-1 pt-2 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
                      {option.group}
                    </div>
                  ) : null}
                  <DropdownMenuRadioItem
                    disabled={option.disabled}
                    value={option.value}
                  >
                    <span className="min-w-0 flex-1 truncate">
                      {option.label}
                    </span>
                    {option.badge ? (
                      <span className="ml-2 shrink-0 rounded-full bg-muted px-1.5 py-0.5 text-2xs text-muted-foreground">
                        {option.badge}
                      </span>
                    ) : null}
                  </DropdownMenuRadioItem>
                </React.Fragment>
              ))}
            </DropdownMenuRadioGroup>
          </div>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
