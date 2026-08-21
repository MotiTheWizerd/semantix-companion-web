const SCROLL_TO_CONVERSATION_END_EVENT = "scrollToConversationEnd";

interface ScrollToConversationEndDetail {
  conversationId: string;
}

const chatScrollEvents = new EventTarget();

export function requestConversationScrollToEnd(conversationId: string): void {
  chatScrollEvents.dispatchEvent(
    new CustomEvent<ScrollToConversationEndDetail>(SCROLL_TO_CONVERSATION_END_EVENT, {
      detail: { conversationId },
    }),
  );
}

export function onConversationScrollToEnd(
  listener: (conversationId: string) => void,
): () => void {
  const handleEvent = (event: Event) => {
    listener((event as CustomEvent<ScrollToConversationEndDetail>).detail.conversationId);
  };

  chatScrollEvents.addEventListener(SCROLL_TO_CONVERSATION_END_EVENT, handleEvent);
  return () => {
    chatScrollEvents.removeEventListener(SCROLL_TO_CONVERSATION_END_EVENT, handleEvent);
  };
}
