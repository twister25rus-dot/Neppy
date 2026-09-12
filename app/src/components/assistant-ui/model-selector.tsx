'use client';

import { cn } from '@/components/assistant-ui/lib/utils';
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from '@/components/assistant-ui/ui/command';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/assistant-ui/ui/popover';
import { useAui } from '@assistant-ui/react';
import { Slider } from '@base-ui/react/slider';
import { cva, type VariantProps } from 'class-variance-authority';
import { CheckIcon, ChevronDownIcon } from 'lucide-react';
import {
  type ComponentPropsWithoutRef,
  createContext,
  memo,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

export type ModelSelectorEffortOption = { id: string; name: string };

/**
 * The ids are the wire values (`reasoning_effort`) and must not change; the
 * names are what the slider reads out. "Low / Med / High" described the knob,
 * not the decision being made with it — the question is whether you want an
 * answer now or a better one.
 */
export const DEFAULT_EFFORT_OPTIONS: readonly ModelSelectorEffortOption[] = [
  { id: 'low', name: 'Quick' },
  { id: 'medium', name: 'Balanced' },
  { id: 'high', name: 'Thorough' },
];

export type ModelOption = {
  id: string;
  name: string;
  description?: string;
  icon?: ReactNode;
  disabled?: boolean;
  /** Extra terms matched by ModelSelector.Search, in addition to id and name. */
  keywords?: readonly string[];
  /**
   * Reasoning effort levels the model supports. Pass `true` for the default
   * low/medium/high levels, or a custom list. Omit for models without
   * configurable reasoning.
   */
  efforts?: boolean | readonly ModelSelectorEffortOption[];
};

function getModelEfforts(
  model: ModelOption | undefined
): readonly ModelSelectorEffortOption[] | undefined {
  if (!model?.efforts) return undefined;
  return model.efforts === true ? DEFAULT_EFFORT_OPTIONS : model.efforts;
}

function resolveEffort(
  efforts: readonly ModelSelectorEffortOption[] | undefined,
  effort: string | undefined
): string | undefined {
  if (effort === undefined) return undefined;
  return efforts?.some(e => e.id === effort) ? effort : undefined;
}

/**
 * Returns the effort id if the given model supports it, otherwise undefined.
 * Effort selection is kept sticky across model switches; this resolves what
 * actually applies to the current model.
 */
export function resolveModelEffort(
  models: readonly ModelOption[],
  modelId: string | undefined,
  effort: string | undefined
): string | undefined {
  return resolveEffort(getModelEfforts(models.find(m => m.id === modelId)), effort);
}

function useControllableState<T>({
  prop,
  defaultProp,
  onChange,
}: {
  prop: T | undefined;
  defaultProp: T | undefined;
  onChange: ((next: T) => void) | undefined;
}) {
  const [internal, setInternal] = useState(defaultProp);
  const isControlled = prop !== undefined;
  const value = isControlled ? prop : internal;
  // Read onChange through a ref so inline callbacks don't recreate the setter
  // (and with it the memoized context value) every render.
  const onChangeRef = useRef(onChange);
  useEffect(() => {
    onChangeRef.current = onChange;
  });
  const setValue = useCallback(
    (next: T) => {
      if (!isControlled) setInternal(next);
      onChangeRef.current?.(next);
    },
    [isControlled]
  );
  return [value, setValue] as const;
}

type ModelSelectorContextValue = {
  models: readonly ModelOption[];
  value: string | undefined;
  setValue: (value: string) => void;
  /** The model matching `value`, derived once for all sub-components. */
  selectedModel: ModelOption | undefined;
  /** The selected model's effort levels, undefined when not configurable. */
  efforts: readonly ModelSelectorEffortOption[] | undefined;
  /** Effort resolved against the selected model's supported levels. */
  effort: string | undefined;
  setEffort: (effort: string) => void;
  setOpen: (open: boolean) => void;
};

const ModelSelectorContext = createContext<ModelSelectorContextValue | null>(null);

function useModelSelectorContext() {
  const ctx = useContext(ModelSelectorContext);
  if (!ctx) {
    throw new Error('ModelSelector sub-components must be used within ModelSelector.Root');
  }
  return ctx;
}

/**
 * The selected model's effort levels and the active selection. Use it to build
 * a custom effort UI inside ModelSelector.Content (e.g. a slider or a shadcn
 * DropdownMenu) when the built-in ModelSelector.Effort layout doesn't fit.
 * `efforts` is undefined for models without configurable reasoning.
 */
export function useModelSelectorEfforts(): {
  efforts: readonly ModelSelectorEffortOption[] | undefined;
  effort: string | undefined;
  setEffort: (effort: string) => void;
} {
  const { efforts, effort, setEffort } = useModelSelectorContext();
  return { efforts, effort, setEffort };
}

export type ModelSelectorRootProps = {
  models: readonly ModelOption[];
  value?: string;
  defaultValue?: string;
  onValueChange?: (value: string) => void;
  effort?: string;
  defaultEffort?: string;
  onEffortChange?: (effort: string) => void;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  children: ReactNode;
};

function ModelSelectorRoot({
  models,
  value: valueProp,
  defaultValue,
  onValueChange,
  effort: effortProp,
  defaultEffort,
  onEffortChange,
  open: openProp,
  defaultOpen,
  onOpenChange,
  children,
}: ModelSelectorRootProps) {
  const [value, setValue] = useControllableState({
    prop: valueProp,
    defaultProp: defaultValue ?? models[0]?.id,
    onChange: onValueChange,
  });
  const [effort, setEffort] = useControllableState({
    prop: effortProp,
    defaultProp: defaultEffort,
    onChange: onEffortChange,
  });
  const [open, setOpen] = useControllableState({
    prop: openProp,
    defaultProp: defaultOpen ?? false,
    onChange: onOpenChange,
  });

  const selectedModel = models.find(m => m.id === value);
  const efforts = getModelEfforts(selectedModel);
  const activeEffort = resolveEffort(efforts, effort);
  const contextValue = useMemo(
    () => ({
      models,
      value,
      setValue,
      selectedModel,
      efforts,
      effort: activeEffort,
      setEffort,
      setOpen,
    }),
    [models, value, setValue, selectedModel, efforts, activeEffort, setEffort, setOpen]
  );

  return (
    <ModelSelectorContext.Provider value={contextValue}>
      <Popover open={open ?? false} onOpenChange={setOpen}>
        {children}
      </Popover>
    </ModelSelectorContext.Provider>
  );
}

export const modelSelectorTriggerVariants = cva(
  "focus-visible:ring-ring/50 flex w-fit items-center justify-between gap-2 overflow-hidden rounded-md text-sm whitespace-nowrap transition-colors outline-none focus-visible:ring-1 disabled:cursor-not-allowed disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5",
  {
    variants: {
      variant: {
        outline: 'border-input hover:bg-accent hover:text-accent-foreground border bg-transparent',
        ghost: 'hover:bg-accent hover:text-accent-foreground',
        muted: 'bg-secondary text-secondary-foreground hover:bg-secondary/80',
      },
      size: { default: 'h-9 px-3 py-2', sm: 'h-8 px-2.5 py-1.5 text-xs', lg: 'h-10 px-4 py-2.5' },
    },
    defaultVariants: { variant: 'outline', size: 'default' },
  }
);

export type ModelSelectorTriggerProps = ComponentPropsWithoutRef<typeof PopoverTrigger> &
  VariantProps<typeof modelSelectorTriggerVariants>;

function ModelSelectorTrigger({
  className,
  variant,
  size,
  children,
  onKeyDown,
  ...props
}: ModelSelectorTriggerProps) {
  const { setOpen } = useModelSelectorContext();

  return (
    <PopoverTrigger
      data-slot="model-selector-trigger"
      data-variant={variant ?? 'outline'}
      data-size={size ?? 'default'}
      role="combobox"
      aria-haspopup="listbox"
      className={cn(modelSelectorTriggerVariants({ variant, size }), className)}
      onKeyDown={e => {
        onKeyDown?.(e);
        if (e.defaultPrevented) return;
        // ARIA combobox: arrows open the listbox from a focused trigger.
        // Popover leaves this to the consumer.
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          e.preventDefault();
          setOpen(true);
        }
      }}
      {...props}>
      {children ?? <ModelSelectorValue />}
      <ChevronDownIcon className="size-4 opacity-50" />
    </PopoverTrigger>
  );
}

export type ModelSelectorValueProps = {
  placeholder?: ReactNode;
  /** Show the active effort level next to the model name. */
  showEffort?: boolean;
  className?: string;
};

function ModelIcon({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        'flex size-3.5 shrink-0 items-center justify-center [&_svg]:size-3.5',
        className
      )}>
      {children}
    </span>
  );
}

function ModelSelectorValue({
  placeholder = 'Select model',
  showEffort = true,
  className,
}: ModelSelectorValueProps) {
  const { selectedModel, efforts, effort } = useModelSelectorContext();

  if (!selectedModel) {
    return (
      <span data-slot="model-selector-value" className={cn('text-muted-foreground', className)}>
        {placeholder}
      </span>
    );
  }

  const effortName =
    showEffort && effort !== undefined ? efforts?.find(e => e.id === effort)?.name : undefined;

  return (
    <span
      data-slot="model-selector-value"
      className={cn('flex min-w-0 items-center gap-2', className)}>
      {selectedModel.icon && <ModelIcon>{selectedModel.icon}</ModelIcon>}
      <span className="truncate font-medium">{selectedModel.name}</span>
      {effortName && (
        <span className="text-muted-foreground min-w-7.5 truncate text-center">{effortName}</span>
      )}
    </span>
  );
}

export type ModelSelectorContentProps = Omit<
  ComponentPropsWithoutRef<typeof PopoverContent>,
  'side'
> & {
  /**
   * Preferred side for the initial placement. Once the popover is open, the
   * rendered side takes over until it closes, so the popup does not jump
   * between sides while filtering resizes the list.
   */
  side?: ComponentPropsWithoutRef<typeof PopoverContent>['side'];
  searchable?: boolean;
};

// Base UI's Popover re-evaluates collision flipping whenever the popup
// resizes, so filtering the list down flips the popup back to the preferred
// side mid-interaction. Base UI only exposes its lazy-flip behavior on the
// Combobox positioner, so mirror it here: feed the rendered side back as the
// preferred side, making the popup keep its side until it no longer fits.
function useLazyFlipSide(): {
  side: ModelSelectorContentProps['side'];
  popupRef: (node: HTMLDivElement | null) => void;
} {
  const [side, setSide] = useState<ModelSelectorContentProps['side']>();
  const observerRef = useRef<MutationObserver | null>(null);
  const popupRef = useCallback((node: HTMLDivElement | null) => {
    observerRef.current?.disconnect();
    observerRef.current = null;
    if (!node) {
      setSide(undefined);
      return;
    }
    const sync = () => {
      const rendered = node.getAttribute('data-side');
      if (rendered) setSide(rendered as ModelSelectorContentProps['side']);
    };
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(node, { attributes: true, attributeFilter: ['data-side'] });
    observerRef.current = observer;
  }, []);
  return { side, popupRef };
}

/**
 * Hidden input that anchors cmdk's keyboard navigation, keeping the list
 * keyboard-operable without a visible search box. ModelSelectorContent renders
 * one automatically when unfiltered.
 */
function ModelSelectorFocusAnchor() {
  return (
    <div className="sr-only">
      <CommandInput readOnly aria-label="Model" />
    </div>
  );
}

function ModelSelectorContent({
  className,
  align = 'start',
  side,
  sideOffset = 6,
  searchable,
  children,
  ...props
}: ModelSelectorContentProps) {
  const { value } = useModelSelectorContext();
  const { side: renderedSide, popupRef } = useLazyFlipSide();
  const unfiltered = searchable === false || (!searchable && children === undefined);

  return (
    <PopoverContent
      ref={popupRef}
      data-slot="model-selector-content"
      align={align}
      side={renderedSide ?? side ?? 'bottom'}
      sideOffset={sideOffset}
      className={cn(
        'bg-popover w-72 min-w-(--anchor-width) overflow-hidden rounded-xl p-0',
        className
      )}
      {...props}>
      <Command
        className="bg-transparent"
        shouldFilter={!unfiltered}
        {...(value !== undefined ? { defaultValue: value } : {})}>
        {unfiltered && <ModelSelectorFocusAnchor />}
        {children ?? (
          <>
            {searchable && <ModelSelectorSearch />}
            <ModelSelectorList />
            <ModelSelectorEffort />
          </>
        )}
      </Command>
    </PopoverContent>
  );
}

export type ModelSelectorSearchProps = ComponentPropsWithoutRef<typeof CommandInput>;

function ModelSelectorSearch({
  placeholder = 'Search models...',
  ...props
}: ModelSelectorSearchProps) {
  return <CommandInput data-slot="model-selector-search" placeholder={placeholder} {...props} />;
}

export type ModelSelectorListProps = ComponentPropsWithoutRef<typeof CommandList>;

function ModelSelectorList({ className, children, ...props }: ModelSelectorListProps) {
  const { models } = useModelSelectorContext();

  return (
    <CommandList
      data-slot="model-selector-list"
      className={cn(
        '[-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden',
        className
      )}
      {...props}>
      {children ?? (
        <>
          <ModelSelectorEmpty />
          <CommandGroup>
            {models.map(model => (
              <ModelSelectorItem key={model.id} model={model} />
            ))}
          </CommandGroup>
        </>
      )}
    </CommandList>
  );
}

export type ModelSelectorEmptyProps = ComponentPropsWithoutRef<typeof CommandEmpty>;

function ModelSelectorEmpty({ children, ...props }: ModelSelectorEmptyProps) {
  return (
    <CommandEmpty data-slot="model-selector-empty" {...props}>
      {children ?? 'No models found.'}
    </CommandEmpty>
  );
}

export type ModelSelectorGroupProps = ComponentPropsWithoutRef<typeof CommandGroup>;

function ModelSelectorGroup(props: ModelSelectorGroupProps) {
  return <CommandGroup data-slot="model-selector-group" {...props} />;
}

export type ModelSelectorSeparatorProps = ComponentPropsWithoutRef<typeof CommandSeparator>;

function ModelSelectorSeparator(props: ModelSelectorSeparatorProps) {
  return <CommandSeparator data-slot="model-selector-separator" {...props} />;
}

export type ModelSelectorItemProps = Omit<ComponentPropsWithoutRef<typeof CommandItem>, 'value'> & {
  model: ModelOption;
};

function ModelSelectorItem({
  model,
  className,
  children,
  onSelect,
  ...props
}: ModelSelectorItemProps) {
  const { value, setValue, setOpen } = useModelSelectorContext();
  const isSelected = value === model.id;

  return (
    <CommandItem
      data-slot="model-selector-item"
      value={model.id}
      keywords={[model.name, ...(model.keywords ?? [])]}
      {...(model.disabled ? { disabled: true } : undefined)}
      onSelect={selectedValue => {
        setValue(model.id);
        setOpen(false);
        onSelect?.(selectedValue);
      }}
      className={cn(
        "relative items-start gap-2 rounded-lg py-2 ps-3 pe-9 [&_svg:not([class*='size-'])]:size-3.5",
        className
      )}
      {...props}>
      {children ?? (
        <>
          {model.icon && <ModelIcon className="mt-[3px]">{model.icon}</ModelIcon>}
          <span className="flex min-w-0 flex-col">
            <span className="truncate font-medium">{model.name}</span>
            {model.description && (
              <span className="text-muted-foreground truncate text-xs">{model.description}</span>
            )}
          </span>
        </>
      )}
      {isSelected && (
        <span className="absolute end-3 top-2.5 flex size-4 items-center justify-center">
          <CheckIcon className="size-4" />
        </span>
      )}
    </CommandItem>
  );
}

export type ModelSelectorEffortProps = ComponentPropsWithoutRef<'div'> & { label?: ReactNode };

function ModelSelectorEffort({
  label = 'Thinking',
  className,
  onKeyDown,
  onKeyDownCapture,
  ...props
}: ModelSelectorEffortProps) {
  const { efforts, effort, setEffort } = useModelSelectorEfforts();

  if (!efforts?.length) return null;

  const lastIndex = efforts.length - 1;
  // An unset or unrecognised effort sits at the first stop rather than leaving
  // the thumb undefined, which would render the track with no handle at all.
  const foundIndex = efforts.findIndex(option => option.id === effort);
  const activeIndex = foundIndex === -1 ? 0 : foundIndex;

  return (
    <div
      data-slot="model-selector-effort"
      className={cn('flex cursor-default flex-col gap-1.5 border-t px-3 py-2.5', className)}
      onKeyDownCapture={e => {
        onKeyDownCapture?.(e);
        if (e.defaultPrevented) return;
        if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
        // The slider claims vertical arrows as well as horizontal ones, so
        // intercept them in capture and hand the keypress to cmdk: the model
        // list owns vertical navigation, and cmdk's Enter is inert while the
        // thumb has focus. Left/Right are left alone — those are the slider's.
        onKeyDown?.(e);
        if (e.defaultPrevented) return;
        const input = e.currentTarget
          .closest('[cmdk-root]')
          ?.querySelector<HTMLInputElement>('[cmdk-input]');
        if (!input) return;
        e.preventDefault();
        e.stopPropagation();
        input.focus();
        input.dispatchEvent(new KeyboardEvent('keydown', e.nativeEvent));
      }}
      onKeyDown={e => {
        if (e.key === 'ArrowUp' || e.key === 'ArrowDown') return;
        onKeyDown?.(e);
        if (e.defaultPrevented) return;
        // cmdk's Command root would claim Home/End to jump the model list;
        // hand focus to the thumb so the slider's own min/max jump runs
        // instead.
        if (e.key === 'Home' || e.key === 'End') {
          e.preventDefault();
          e.stopPropagation();
          e.currentTarget.querySelector<HTMLElement>('[role="slider"]')?.focus();
        }
      }}
      {...props}>
      <span className="text-muted-foreground text-xs">{label}</span>
      <Slider.Root
        value={activeIndex}
        min={0}
        max={lastIndex}
        step={1}
        // Base UI hands back a bare number when it was given one.
        onValueChange={next => {
          const option = efforts[Math.round(next)];
          if (option) setEffort(option.id);
        }}
        aria-label={typeof label === 'string' ? label : 'Reasoning effort'}
        className="w-full">
        <Slider.Control className="flex h-4 w-full touch-none items-center select-none">
          <Slider.Track className="bg-muted relative h-1 w-full rounded-full">
            <Slider.Indicator className="bg-primary rounded-full" />
            <Slider.Thumb
              // The stop names are the visible scale, so read those out rather
              // than "1", which says nothing about what the position buys.
              getAriaValueText={(_formatted, value) =>
                efforts[Math.round(value)]?.name ?? String(value)
              }
              className="focus-visible:ring-ring/50 bg-primary block size-3.5 rounded-full shadow-sm transition-transform outline-none hover:scale-110 focus-visible:ring-2"
            />
          </Slider.Track>
        </Slider.Control>
      </Slider.Root>
      {/* The scale itself. `justify-between` puts three labels under the three
          stops the thumb can occupy; the active one is the only one at full
          contrast, so the current setting is readable without a second row. */}
      <div aria-hidden="true" className="flex justify-between gap-1 text-[10px]">
        {efforts.map((option, index) => (
          <span
            key={option.id}
            className={cn(
              'transition-colors',
              index === activeIndex ? 'text-foreground font-medium' : 'text-muted-foreground'
            )}>
            {option.name}
          </span>
        ))}
      </div>
    </div>
  );
}

export type ModelSelectorProps = Omit<ModelSelectorRootProps, 'children'> &
  VariantProps<typeof modelSelectorTriggerVariants> & {
    /** Render a search input above the model list. */
    searchable?: boolean;
    /** Alignment of the dropdown relative to the trigger. Use `"end"` when the
     * trigger sits at the right edge of its container. */
    align?: ModelSelectorContentProps['align'];
    className?: string;
    contentClassName?: string;
  };

/** Registers the selection with assistant-ui's ModelContext system. The
 * context's effort is already resolved against the selected model. */
function ModelSelectorModelContext() {
  const { value, effort } = useModelSelectorContext();
  const api = useAui();

  useEffect(() => {
    if (value === undefined) return;
    const config = {
      config: {
        modelName: value,
        ...(effort !== undefined ? { reasoningEffort: effort } : undefined),
      },
    };
    return api.modelContext.register({ getModelContext: () => config });
  }, [api, value, effort]);

  return null;
}

const ModelSelectorImpl = ({
  searchable,
  variant,
  size,
  align,
  className,
  contentClassName,
  ...rootProps
}: ModelSelectorProps) => {
  return (
    <ModelSelectorRoot {...rootProps}>
      <ModelSelectorModelContext />
      <ModelSelectorTrigger variant={variant} size={size} className={className} />
      <ModelSelectorContent
        {...(align !== undefined ? { align } : {})}
        className={contentClassName}
        searchable={searchable ?? false}
      />
    </ModelSelectorRoot>
  );
};

type ModelSelectorComponent = typeof ModelSelectorImpl & {
  displayName?: string;
  Root: typeof ModelSelectorRoot;
  Trigger: typeof ModelSelectorTrigger;
  Value: typeof ModelSelectorValue;
  Content: typeof ModelSelectorContent;
  Search: typeof ModelSelectorSearch;
  FocusAnchor: typeof ModelSelectorFocusAnchor;
  List: typeof ModelSelectorList;
  Empty: typeof ModelSelectorEmpty;
  Group: typeof ModelSelectorGroup;
  Separator: typeof ModelSelectorSeparator;
  Item: typeof ModelSelectorItem;
  Effort: typeof ModelSelectorEffort;
};

const ModelSelector = memo(ModelSelectorImpl) as unknown as ModelSelectorComponent;

ModelSelector.displayName = 'ModelSelector';
ModelSelector.Root = ModelSelectorRoot;
ModelSelector.Trigger = ModelSelectorTrigger;
ModelSelector.Value = ModelSelectorValue;
ModelSelector.Content = ModelSelectorContent;
ModelSelector.Search = ModelSelectorSearch;
ModelSelector.FocusAnchor = ModelSelectorFocusAnchor;
ModelSelector.List = ModelSelectorList;
ModelSelector.Empty = ModelSelectorEmpty;
ModelSelector.Group = ModelSelectorGroup;
ModelSelector.Separator = ModelSelectorSeparator;
ModelSelector.Item = ModelSelectorItem;
ModelSelector.Effort = ModelSelectorEffort;

export {
  ModelSelector,
  ModelSelectorRoot,
  ModelSelectorTrigger,
  ModelSelectorValue,
  ModelSelectorContent,
  ModelSelectorSearch,
  ModelSelectorFocusAnchor,
  ModelSelectorList,
  ModelSelectorEmpty,
  ModelSelectorGroup,
  ModelSelectorSeparator,
  ModelSelectorItem,
  ModelSelectorEffort,
};
